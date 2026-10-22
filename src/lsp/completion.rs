//! Bounded pattern continuations, input templates and current-revision history.
//!
//! These are editing suggestions, not successful parse candidates. Expected
//! types use native contracts; execution scopes and addon guards are not guessed.
use super::{positions::LineIndex, public_data::variable_details};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionList, CompletionTextEdit, Documentation,
    InsertTextFormat, Position, TextEdit,
};
use parser_wasm::{DocumentParseResult, host::AstNode};
use skript_parser::{
    FunctionDefinition, FunctionScope, RawNodeId, RawNodeKind, RawTriviaKind, TextRange,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};
use syntax_pattern_parser::syntax::{PatternElement, SpannedPatternElement};
use syntaxes::{Catalog, ClassName, Syntax, SyntaxKind};

mod continuation;
mod cursor;
mod finite;
mod parameters;
mod prepared;
mod ranking;
pub(super) use prepared::{OwnerAttempt, PreparedDocument, PreparedQuery, PreparedSection};
mod signature;
mod structure;
mod typing;
#[cfg(test)]
mod typing_tests;
pub(super) use signature::{SignatureCapabilities, SignatureQuery};
use structure::{EntryScope, Scope};

// Bound transfer and retained menu state; incomplete results are recomputed as
// the prefix narrows. All kinds use the same ranking before this limit.
const MAX_ITEMS: usize = 512;
const MAX_VARIANTS: usize = 16;
const MAX_TEMPLATE_BYTES: usize = 1024;
const MAX_SYNTAX_SOURCE_BYTES: usize = 256 * 1024;
const MAX_CONTINUATION_BYTES: usize = 4096;

#[derive(Clone)]
enum Part {
    Text(String),
    Slot(String),
}
#[derive(Clone)]
struct Template {
    finite_capture: bool,
    documentation: Option<Arc<str>>,
    kind: SyntaxKind,
    label: String,
    detail: String,
    identity: String,
    leading: String,
    folded: String,
    parts: Vec<Part>,
}
struct Pattern {
    documentation: Option<Arc<str>>,
    kind: SyntaxKind,
    label: String,
    detail: String,
    identity: String,
    graph: continuation::Graph,
}
#[derive(Clone)]
struct Function {
    name: String,
    identity: String,
    detail: String,
    parameters: Vec<(String, String, bool, bool)>,
    returns_value: bool,
    definition: Option<Arc<FunctionDefinition>>,
}
impl Function {
    fn shape(&self) -> Vec<(&str, bool)> {
        self.shape_iter().collect()
    }
    fn shape_iter(&self) -> impl Iterator<Item = (&str, bool)> {
        self.parameters
            .iter()
            .map(|(_, class, single, _)| (class.as_str(), *single))
    }
    fn from_definition(definition: &FunctionDefinition) -> Self {
        Self {
            name: definition.name.clone(),
            identity: definition.registration_id.clone(),
            detail: "Function / script declaration".into(),
            parameters: definition
                .parameters
                .iter()
                .map(|p| {
                    (
                        p.name.clone(),
                        p.parameter_type.0.clone(),
                        p.single,
                        p.optional,
                    )
                })
                .collect(),
            returns_value: definition.return_type.is_some(),
            definition: Some(Arc::new(definition.clone())),
        }
    }
}

pub(super) struct CatalogIndex {
    templates: Vec<Template>,
    patterns: Vec<Pattern>,
    functions: Vec<Function>,
    limited: bool,
    typing: Option<typing::Index>,
    raw_options: skript_parser::RawTreeOptions,
    documentation_format: lsp_types::MarkupKind,
}
impl CatalogIndex {
    pub fn with_documentation_format(
        catalog: &Arc<Catalog>,
        raw_options: skript_parser::RawTreeOptions,
        documentation_format: lsp_types::MarkupKind,
    ) -> Self {
        let mut index = Self {
            templates: Vec::new(),
            patterns: Vec::new(),
            functions: Vec::new(),
            limited: false,
            raw_options,
            documentation_format,
            typing: Some(typing::Index::new(Arc::clone(catalog))),
        };
        let mut function_shapes = HashSet::new();
        // Iterate the actual Catalog, preserving registration and pattern order.
        for syntax in catalog.syntaxes() {
            if let Syntax::Function(function) = syntax {
                let mut candidate =
                    Function::from_definition(&FunctionDefinition::from_catalog(function));
                let shape = (
                    candidate.name.clone(),
                    candidate
                        .parameters
                        .iter()
                        .map(|(_, class, single, _)| (class.clone(), *single))
                        .collect::<Vec<_>>(),
                );
                // Match parser lookup order even when the first definition is void.
                if function_shapes.insert(shape) {
                    candidate.detail = format!("Function / {}", function.addon.name);
                    index.functions.push(candidate);
                }
                continue;
            }
            let Some(common) = syntax.common() else {
                continue;
            };
            let documentation: Arc<str> = super::hover_text::render(
                common.documentation.name.as_deref().unwrap_or("Syntax"),
                &[("Addon", common.addon.name.clone())],
                Some(&common.documentation),
                &index.documentation_format,
            )
            .into();
            for (pattern_index, pattern) in common.patterns.iter().enumerate() {
                if pattern.source.len() > MAX_TEMPLATE_BYTES {
                    index.limited = true;
                    index.typing.as_mut().unwrap().complete_patterns = false;
                    continue;
                }
                let detail = match common
                    .documentation
                    .name
                    .as_deref()
                    .filter(|name| !name.is_empty())
                {
                    Some(name) => format!("{:?} / {name} / {}", syntax.kind(), common.addon.name),
                    None => format!("{:?} / {}", syntax.kind(), common.addon.name),
                };
                let identity = format!("{}:{pattern_index}", common.registration_id.as_str());
                if let Syntax::Expression(expression) = syntax {
                    index.typing.as_mut().unwrap().insert(&identity, expression);
                }
                let event_prefix = matches!(syntax, Syntax::Event(event) if event.has_on_prefix);
                if let Some(graph) = continuation::Graph::with_prefix(
                    if event_prefix { "on " } else { "" },
                    &pattern.parsed.elements,
                ) {
                    index.patterns.push(Pattern {
                        documentation: Some(Arc::clone(&documentation)),
                        kind: syntax.kind(),
                        label: pattern.source.clone(),
                        detail: detail.clone(),
                        identity: identity.clone(),
                        graph,
                    });
                } else {
                    index.limited = true;
                    index.typing.as_mut().unwrap().complete_patterns = false;
                }
                let variants = variants(
                    &pattern.parsed.elements,
                    0,
                    &mut index.limited,
                    catalog.plural_rules(),
                );
                let mut seen = HashSet::new();
                for mut parts in variants {
                    if let Syntax::Event(event) = syntax {
                        if event.has_on_prefix {
                            parts.insert(0, Part::Text("on ".into()));
                        }
                    }
                    if matches!(
                        syntax.kind(),
                        SyntaxKind::Event | SyntaxKind::Section | SyntaxKind::Structure
                    ) {
                        parts.push(Part::Text(":".into()));
                    }
                    parts = normalize_parts(parts);
                    let leading = parts
                        .iter()
                        .take_while(|p| matches!(p, Part::Text(_)))
                        .map(|p| match p {
                            Part::Text(t) => t.as_str(),
                            _ => unreachable!(),
                        })
                        .collect::<String>();
                    if leading.trim().is_empty() || !seen.insert(render(&parts, true)) {
                        continue;
                    }
                    index.templates.push(Template {
                        finite_capture: false,
                        documentation: Some(Arc::clone(&documentation)),
                        kind: syntax.kind(),
                        label: pattern.source.clone(),
                        detail: detail.clone(),
                        identity: identity.clone(),
                        folded: leading.to_lowercase(),
                        leading,
                        parts,
                    });
                }
            }
        }
        index
    }
}

fn variants(
    elements: &[SpannedPatternElement],
    depth: usize,
    limited: &mut bool,
    plural_rules: &syntax_pattern_parser::syntax::PluralRules,
) -> Vec<Vec<Part>> {
    if depth > 32 {
        *limited = true;
        return Vec::new();
    }
    let mut output = vec![Vec::new()];
    for element in elements {
        let choices = match &element.value {
            PatternElement::Literal(text) => vec![vec![Part::Text(text.clone())]],
            PatternElement::TypeExpr(types) => vec![vec![Part::Slot(
                types.display_with(plural_rules).to_string(),
            )]],
            PatternElement::Regex(_) => vec![vec![Part::Slot("value".into())]],
            PatternElement::Group(group) => variants(group, depth + 1, limited, plural_rules),
            PatternElement::Choice(branches) => {
                let mut choices = Vec::new();
                for branch in branches {
                    choices.extend(variants(branch, depth + 1, limited, plural_rules));
                    if choices.len() > MAX_VARIANTS {
                        choices.truncate(MAX_VARIANTS);
                        *limited = true;
                        break;
                    }
                }
                choices
            }
            PatternElement::Option(option) => {
                let mut choices = vec![Vec::new()];
                choices.extend(variants(option, depth + 1, limited, plural_rules));
                choices
            }
            PatternElement::ParseTag(_) | PatternElement::ParseMark(_) | PatternElement::Empty => {
                vec![Vec::new()]
            }
        };
        let mut next = Vec::new();
        'products: for prefix in &output {
            for choice in &choices {
                if next.len() == MAX_VARIANTS {
                    *limited = true;
                    break 'products;
                }
                let mut parts = prefix.clone();
                parts.extend(choice.clone());
                if parts
                    .iter()
                    .map(|p| match p {
                        Part::Text(t) | Part::Slot(t) => t.len(),
                    })
                    .sum::<usize>()
                    > MAX_TEMPLATE_BYTES
                {
                    *limited = true;
                    continue;
                }
                next.push(parts);
            }
        }
        output = next;
    }
    output
}
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('$', "\\$")
        .replace('}', "\\}")
}
fn normalize_parts(parts: Vec<Part>) -> Vec<Part> {
    let mut output = Vec::new();
    for part in parts {
        match part {
            Part::Text(text) => match output.last_mut() {
                Some(Part::Text(previous)) => previous.push_str(&text),
                _ => output.push(Part::Text(text)),
            },
            part => output.push(part),
        }
    }
    for part in &mut output {
        if let Part::Text(text) = part {
            let leading = text.starts_with(char::is_whitespace);
            let trailing = text.ends_with(char::is_whitespace);
            let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
            *text = if normalized.is_empty() {
                if leading { " ".into() } else { String::new() }
            } else {
                format!(
                    "{}{}{}",
                    if leading { " " } else { "" },
                    normalized,
                    if trailing { " " } else { "" }
                )
            };
        }
    }
    if let Some(Part::Text(first)) = output.first_mut() {
        *first = first.trim_start().to_owned();
    }
    if let Some(Part::Text(last)) = output.last_mut() {
        *last = last.trim_end().to_owned();
    }
    output
}
fn preview(parts: &[Part]) -> String {
    parts
        .iter()
        .map(|part| match part {
            Part::Text(text) => text.clone(),
            Part::Slot(label) => format!("%{label}%"),
        })
        .collect::<String>()
}
fn render(parts: &[Part], snippets: bool) -> String {
    let mut output = String::new();
    let mut slot = 0;
    for part in parts {
        match part {
            Part::Text(text) => {
                output.push_str(&if snippets { escape(text) } else { text.clone() })
            }
            Part::Slot(label) if snippets => {
                slot += 1;
                output.push_str(&format!("${{{slot}:{}}}", escape(label)));
            }
            Part::Slot(_) => break,
        }
    }
    output
}

#[derive(Clone)]
struct Line {
    node: Option<RawNodeId>,
    scope: Scope,
    range: TextRange,
    code_start: usize,
    parent: Option<RawNodeId>,
    ancestors: Vec<RawNodeId>,
    root: bool,
    section: bool,
    blocked: bool,
    comments: Vec<TextRange>,
}
struct Variable {
    source: String,
    parent: Option<RawNodeId>,
    local: bool,
    observed: usize,
    canonical_name: Option<String>,
}
pub(super) struct Document {
    source: Arc<str>,
    lines: LineIndex,
    context: BTreeMap<u32, Line>,
    entry_scopes: Vec<EntryScope>,
    variables: Vec<Variable>,
    parameters: HashMap<RawNodeId, Vec<parameters::Parameter>>,
    functions: Vec<Function>,
    index: Arc<CatalogIndex>,
    blocked: Vec<TextRange>,
    function_namespace_complete: bool,
    type_context_exact: bool,
    accepted_slots: HashMap<(usize, usize), Vec<skript_parser::ExpressionExpectedType>>,
    pub(super) semantic_types: Option<Arc<super::semantic::Document>>,
}

fn exact_native(span: &skript_parser::MappedSpan) -> Option<TextRange> {
    let [origin] = span.origins.as_slice() else {
        return None;
    };
    (origin.kind == skript_parser::OriginKind::Exact).then_some(origin.original_range)
}
fn exact_node(node: &AstNode, source: &str) -> Option<TextRange> {
    let [origin] = node.span.origins.as_slice() else {
        return None;
    };
    if origin.kind != parser_wasm::bindings::nlaocs::skript_parser_addon::types::OriginKind::Exact {
        return None;
    }
    let range = TextRange::new(
        origin.original_range.start.try_into().ok()?,
        origin.original_range.end.try_into().ok()?,
    );
    (source.get(range.start..range.end)? == node.text).then_some(range)
}
impl Document {
    /// Lexical syntax suggestions only. No old AST, scope, or reference data is reused.
    pub fn from_source(source: Arc<str>, index: Arc<CatalogIndex>) -> Result<Self, String> {
        if source.len() > MAX_SYNTAX_SOURCE_BYTES {
            return Err("syntax completion source exceeds native context budget".into());
        }
        let mapped = skript_parser::MappedSource::identity(Arc::clone(&source));
        let tree = skript_parser::parse_raw_tree(&mapped, index.raw_options);
        let lines = LineIndex::new(&source);
        let context = raw_context(&tree, &source, &lines, false)?;
        Ok(Self {
            source,
            lines,
            context,
            entry_scopes: Vec::new(),
            variables: Vec::new(),
            parameters: HashMap::new(),
            functions: Vec::new(),
            index,
            blocked: Vec::new(),
            function_namespace_complete: true,
            type_context_exact: false,
            accepted_slots: HashMap::new(),
            semantic_types: None,
        })
    }

    pub fn syntax_at(&self, position: Position, snippets: bool) -> Result<CompletionList, String> {
        self.at_with_references(position, snippets, false)
    }
    /// A source-confirmed string/comment cannot gain candidates from references.
    pub fn suppresses_completion(&self, position: Position) -> bool {
        let Some(offset) = self.lines.offset(&self.source, position) else {
            return false;
        };
        let Some(line) = self.context.get(&position.line) else {
            return false;
        };
        if offset < line.code_start || offset > line.range.end {
            return false;
        }
        let line = &self.structure_query_line(line, offset);
        line.blocked
            || line
                .comments
                .iter()
                .any(|r| r.start <= offset && offset <= r.end)
            || !matches!(line.scope, Scope::Pending)
                && matches!(
                    lexical_context(&self.source[line.code_start..offset]),
                    Lexical::String | Lexical::Comment
                )
    }
    pub fn from_result(
        result: &DocumentParseResult,
        source: Arc<str>,
        index: Arc<CatalogIndex>,
    ) -> Result<Self, String> {
        if source.as_ref() != result.source.original() {
            return Err("completion source differs from parsed revision".into());
        }
        let lines = LineIndex::new(&source);
        let blocked = result
            .unresolved_spans()
            .flat_map(|span| span.origins.iter())
            .map(|o| {
                Ok(TextRange::new(
                    usize::try_from(o.original_range.start).map_err(|_| "fault range overflow")?,
                    usize::try_from(o.original_range.end).map_err(|_| "fault range overflow")?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        for range in &blocked {
            lines.range(&source, *range)?;
        }
        let mut context = raw_context(&result.raw_tree, &source, &lines, true)?;
        let entry_scopes = structure::apply(result, &mut context);
        let nodes = result
            .ast
            .nodes
            .iter()
            .map(|node| (node.id, node))
            .collect::<HashMap<_, _>>();
        let mut reachable = HashSet::new();
        let mut pending = result.ast.roots.clone();
        while let Some(id) = pending.pop() {
            if !reachable.insert(id) {
                continue;
            }
            let node = nodes.get(&id).ok_or("missing completion AST node")?;
            pending.extend(node.children.iter().copied());
        }
        let witnesses = result.semantic_witnesses();
        let accepted_slots =
            typing::accepted_slots(&witnesses, &nodes, &reachable, &index, &source, &blocked);
        let mut variables = Vec::new();
        let mut parameters = HashMap::new();
        for id in reachable {
            let node = nodes[&id];
            if node.kind == parser_wasm::host::SyntaxKind::Structure {
                if let Some(header) = exact_node(node, &source)
                    .filter(|range| !blocked.iter().any(|b| overlaps(*b, *range)))
                {
                    let row = lines.position(&source, header.start)?.line;
                    if let Some(owner) = context
                        .get(&row)
                        .filter(|line| line.root && !line.blocked && header.end <= line.range.end)
                        .and_then(|line| line.node)
                    {
                        if let Some(bindings) = super::public_data::parameter_bindings(
                            node.metadata
                                .iter()
                                .filter(|entry| {
                                    entry.key == "skript.semantic.parameters"
                                        && entry.owner_component_id.as_deref().is_some_and(
                                            |owner| witnesses.provider_valid(node, owner),
                                        )
                                })
                                .map(|entry| (entry.key.as_str(), entry.value.as_str())),
                        ) {
                            if let Some(declarations) = parameters::project(
                                bindings,
                                &source,
                                header,
                                node.span.virtual_range.start,
                            ) {
                                parameters.insert(owner, declarations);
                            }
                        }
                    }
                }
            }
            if node.kind
                != parser_wasm::bindings::nlaocs::skript_parser_addon::types::SyntaxKind::Expression
                || node.summary.as_ref().is_none_or(|s| s.kind != "variable")
                || !node.children.is_empty()
            {
                continue;
            }
            let Some((scope, name)) = variable_details(node) else {
                continue;
            };
            if name.contains(['\r', '\n']) {
                continue;
            }
            let Some(range) = exact_node(node, &source) else {
                continue;
            };
            if blocked.iter().any(|b| overlaps(*b, range)) {
                continue;
            }
            let row = lines.position(&source, range.start)?.line;
            let Some(line) = context.get(&row) else {
                continue;
            };
            if line.blocked || range.end > line.range.end {
                continue;
            }
            variables.push(Variable {
                source: format!("{{{}{name}}}", if scope == "local" { "_" } else { "" }),
                parent: line.parent,
                local: scope == "local",
                observed: range.start,
                canonical_name: witnesses
                    .unique_issuer(node)
                    .filter(|issuer| witnesses.provider_valid(node, issuer))
                    .and_then(|_| {
                        let mut records = node
                            .summary
                            .as_ref()?
                            .public_data
                            .iter()
                            .filter(|r| r.schema_id == "nlaocs.skript.variable-identity");
                        let record = records.next()?;
                        if records.next().is_some() || record.schema_version != 1 {
                            return None;
                        }
                        let identity: super::public_data::VariableIdentity =
                            serde_json::from_str(&record.json).ok()?;
                        (identity.scope == "local"
                            && identity.resolution == "static"
                            && !identity.list
                            && !identity.name.is_empty())
                        .then_some(identity.name)
                    }),
            });
        }
        variables.sort_by_key(|v| v.observed);
        let mut functions = Vec::new();
        let mut names = HashSet::new();
        for definition in result.functions.definitions() {
            if names.insert(definition.name.clone()) {
                functions.extend(
                    result
                        .functions
                        .lookup_functions(&definition.name, FunctionScope::Local)
                        .iter()
                        .map(Function::from_definition),
                );
            }
        }
        Ok(Self {
            source,
            lines,
            context,
            entry_scopes,
            variables,
            parameters,
            functions,
            index,
            blocked,
            function_namespace_complete: result.function_namespace_complete,
            type_context_exact: result.source.original() == result.source.virtual_source(),
            accepted_slots,
            semantic_types: None,
        })
    }

    pub fn at(&self, position: Position, snippets: bool) -> Result<CompletionList, String> {
        self.at_with_references(position, snippets, true)
    }
    fn at_with_references(
        &self,
        position: Position,
        snippets: bool,
        references: bool,
    ) -> Result<CompletionList, String> {
        let certified_types = self.index.typing.as_ref().is_some_and(|i| i.current());
        let offset = self
            .lines
            .offset(&self.source, position)
            .ok_or("invalid completion UTF-16 position")?;
        let empty = || CompletionList {
            is_incomplete: false,
            items: Vec::new(),
        };
        let fallback;
        let line = if let Some(line) = self.context.get(&position.line) {
            line
        } else {
            let start = self
                .lines
                .offset(&self.source, Position::new(position.line, 0))
                .unwrap();
            let text = self.source[start..]
                .split('\n')
                .next()
                .unwrap()
                .trim_end_matches('\r');
            if !text.chars().all(char::is_whitespace) {
                return Ok(empty());
            }
            fallback = Line {
                node: None,
                scope: if text.is_empty() {
                    Scope::Statements
                } else {
                    Scope::Pending
                },
                range: TextRange::new(start, start + text.len()),
                code_start: start + text.len(),
                parent: None,
                ancestors: Vec::new(),
                root: text.is_empty(),
                section: false,
                blocked: false,
                comments: Vec::new(),
            };
            &fallback
        };
        if line.blocked
            || offset < line.code_start
            || line
                .comments
                .iter()
                .any(|r| r.start <= offset && offset <= r.end)
            || line
                .comments
                .iter()
                .chain(&self.blocked)
                .any(|r| r.start <= offset && offset < r.end || r.is_empty() && r.start == offset)
        {
            return Ok(empty());
        }
        let query_line = self.structure_query_line(line, offset);
        let line = &query_line;
        if matches!(line.scope, Scope::Pending | Scope::Raw) {
            return Ok(empty());
        }
        if let Scope::Entries(index) = line.scope {
            return self.entry_completion(line, offset, index, snippets);
        }
        let prefix = &self.source[line.code_start..offset];
        let folded_prefix = prefix.to_lowercase();
        let state = lexical_context(prefix);
        if matches!(state, Lexical::String | Lexical::Comment) {
            return Ok(empty());
        }
        let finite_captures_current = self
            .index
            .typing
            .as_ref()
            .is_some_and(|i| i.finite_captures_current());
        let word_start = prefix
            .char_indices()
            .rev()
            .find(|(_, c)| !word(*c))
            .map_or(line.code_start, |(i, c)| line.code_start + i + c.len_utf8());
        let word_end = self.source[offset..line.range.end]
            .char_indices()
            .find(|(_, c)| !word(*c))
            .map_or(line.range.end, |(i, _)| offset + i);
        let variable_start = if let Lexical::Variable(start) = state {
            Some(line.code_start + start)
        } else {
            None
        };
        let expression = matches!(line.scope, Scope::Expressions)
            || variable_start.is_some()
            || !line.root
                && (!prefix.is_empty()
                    && (prefix.ends_with(char::is_whitespace)
                        || prefix.split_whitespace().count() > 1
                        || !self.index.templates.iter().any(|t| {
                            matches!(
                                t.kind,
                                SyntaxKind::Effect | SyntaxKind::Section | SyntaxKind::Condition
                            ) && t.folded.starts_with(&folded_prefix)
                        })));
        let type_edit_start = variable_start.unwrap_or(word_start);
        let type_edit_end = if let Some(start) = variable_start {
            match cursor::variable_edit_end(&self.source[start..line.range.end], offset - start) {
                Ok(Some(close)) => start + close,
                Ok(None) => line.range.end,
                Err(()) => return Ok(empty()),
            }
        } else {
            word_end
        };
        let type_edit = TextRange::new(type_edit_start, type_edit_end);
        // Variable replacements use their authenticated lookup proof below;
        // grammar-hole expectations are unused when only variables are shown.
        let expected_types = (expression && variable_start.is_none())
            .then(|| self.expected_at(line, type_edit, references))
            .flatten();
        let expected = expected_types.as_deref();
        let mut completion_type_budget = continuation::Budget::new();
        let mut items = ranking::Pool::new(&self.source, &self.lines, offset, expected.is_some());
        let mut frontier_limited = prefix.len() > MAX_CONTINUATION_BYTES;
        if variable_start.is_none()
            && word_start > line.code_start
            && !frontier_limited
            && !matches!(line.scope, Scope::Expressions)
        {
            let committed = &self.source[line.code_start..word_start];
            let query = &self.source[word_start..offset];
            let folded_query = query.to_lowercase();
            let mut continuations = Vec::new();
            let mut budget = continuation::Budget::new();
            for (order, pattern) in self.index.patterns.iter().enumerate() {
                if budget.exhausted() {
                    frontier_limited = true;
                    break;
                }
                let allowed = if line.root {
                    matches!(pattern.kind, SyntaxKind::Event | SyntaxKind::Structure)
                } else {
                    matches!(
                        pattern.kind,
                        SyntaxKind::Effect | SyntaxKind::Condition | SyntaxKind::Section
                    )
                };
                if !allowed
                    || matches!(line.scope, Scope::Conditions)
                        && pattern.kind != SyntaxKind::Condition
                {
                    continue;
                }
                let frontier = pattern
                    .graph
                    .next_words(committed, &folded_query, &mut budget);
                frontier_limited |= frontier.limited;
                for (keyword, fixed, slots) in frontier.words {
                    continuations.push((fixed, slots, order, keyword, pattern));
                }
            }
            continuations.sort_by(|a, b| {
                b.0.cmp(&a.0)
                    .then(a.1.cmp(&b.1))
                    // With equal structural evidence, complete the nearest
                    // fixed word first (e.g. `to` before `towards`). This is
                    // editing relevance, not parser or addon precedence.
                    .then(a.3.chars().count().cmp(&b.3.chars().count()))
                    .then(a.3.cmp(&b.3))
                    .then(a.2.cmp(&b.2))
            });
            for (_fixed, _slots, _, keyword, pattern) in continuations {
                let detail = format!(
                    "{}; alternative grammar continuation (arguments unverified)",
                    pattern.detail
                );
                let mut item = self.item(
                    &keyword,
                    &keyword,
                    &detail,
                    &pattern.identity,
                    CompletionItemKind::KEYWORD,
                    TextRange::new(word_start, word_end),
                    false,
                )?;
                item.data = Some(
                    serde_json::json!({"syntaxKind":format!("{:?}", pattern.kind),"continuation":true,"syntaxIdentity":pattern.identity}),
                );
                item.documentation = Some(super::hover_text::completion(
                    &pattern.label,
                    None,
                    &detail,
                    &pattern.identity,
                    pattern.documentation.as_deref(),
                    &self.index.documentation_format,
                ));
                items.push(item);
            }
        }
        // Frontier exploration is bounded independently of final menu selection.
        // Whitespace may start an argument while still matching fixed words in
        // the current statement. Keep those matches ahead of generic expressions.
        if expression
            && !line.root
            && variable_start.is_none()
            && !matches!(line.scope, Scope::Expressions)
        {
            for template in &self.index.templates {
                if matches!(
                    template.kind,
                    SyntaxKind::Effect | SyntaxKind::Section | SyntaxKind::Condition
                ) && (!matches!(line.scope, Scope::Conditions)
                    || template.kind == SyntaxKind::Condition)
                    && template.folded.starts_with(&folded_prefix)
                {
                    items.push(
                        self.template_item(
                            template,
                            prefix,
                            line.code_start,
                            word_end,
                            line,
                            snippets,
                        )?
                        .0,
                    );
                }
            }
        }
        if expression {
            let start = variable_start.unwrap_or(word_start);
            let mut end = word_end;
            if let Some(start) = variable_start {
                match cursor::variable_edit_end(&self.source[start..line.range.end], offset - start)
                {
                    Ok(Some(close)) => end = start + close,
                    Ok(None) => end = line.range.end,
                    Err(()) => return Ok(empty()),
                }
            }
            let query = &self.source[start..offset];
            let mut seen = HashSet::new();
            for owner in &line.ancestors {
                for parameter in self.parameters.get(owner).into_iter().flatten() {
                    let search = if variable_start.is_some() {
                        parameter.source.as_str()
                    } else {
                        parameter.source.trim_start_matches('{')
                    };
                    if starts(search, query)
                        && (parameter.list
                            || !self.incompatible_variable(
                                &parameter.name,
                                &parameter.source,
                                type_edit,
                                &mut completion_type_budget,
                            ))
                        && seen.insert(parameter.source.as_str())
                    {
                        let mut item = self.item(
                            &parameter.source,
                            &parameter.source,
                            &format!(
                                "declared parameter: {} ({})",
                                parameter.class_name,
                                if parameter.single { "single" } else { "plural" }
                            ),
                            &format!("parameter:{}:{}", owner.get(), parameter.source),
                            CompletionItemKind::VARIABLE,
                            TextRange::new(start, end),
                            false,
                        )?;
                        item.data = Some(serde_json::json!({"declaredType": {
                            "className": parameter.class_name, "single": parameter.single,
                        }, "parameterDeclaration": {
                            "rawNodeId": owner.get(), "list": parameter.list,
                            "name": parameter.name, "scope": "local",
                            "range": self.lines.range(&self.source, parameter.range)?,
                        }}));
                        items.push(item);
                    }
                }
            }
            for variable in self.variables.iter().filter(|_| references) {
                if variable.local
                    && (variable.observed >= offset
                        || variable.parent.is_none()
                        || !line.ancestors.contains(&variable.parent.unwrap()))
                {
                    continue;
                }
                let search = if variable_start.is_some() {
                    variable.source.as_str()
                } else {
                    variable.source.trim_start_matches('{')
                };
                if starts(search, query)
                    && !variable.canonical_name.as_ref().is_some_and(|name| {
                        self.incompatible_variable(
                            name,
                            &variable.source,
                            type_edit,
                            &mut completion_type_budget,
                        )
                    })
                    && seen.insert(variable.source.as_str())
                {
                    items.push(self.item(
                        &variable.source,
                        &variable.source,
                        "variable reference history; binding and type are unknown",
                        &format!("variable:{}", variable.source),
                        CompletionItemKind::VARIABLE,
                        TextRange::new(start, end),
                        false,
                    )?);
                }
            }
            if variable_start.is_none() && references && self.function_namespace_complete {
                let query = &self.source[word_start..offset];
                for (index, function) in self
                    .functions
                    .iter()
                    .chain(&self.index.functions)
                    .enumerate()
                {
                    if !function.returns_value || !starts(&function.name, query) {
                        continue;
                    }
                    if index >= self.functions.len()
                        && self.functions.iter().any(|f| {
                            f.name == function.name && f.shape_iter().eq(function.shape_iter())
                        })
                    {
                        continue;
                    }
                    let existing_call = self.source[word_end..line.range.end]
                        .trim_start()
                        .starts_with('(');
                    let use_snippets = snippets && !existing_call;
                    let insertion = if existing_call {
                        function.name.clone()
                    } else {
                        let required = function
                            .parameters
                            .iter()
                            .filter(|p| !p.3)
                            .collect::<Vec<_>>();
                        if use_snippets {
                            format!(
                                "{}({})",
                                escape(&function.name),
                                required
                                    .iter()
                                    .enumerate()
                                    .map(|(i, p)| format!("${{{}:{}}}", i + 1, escape(&p.0)))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        } else if required.is_empty() {
                            format!("{}()", function.name)
                        } else {
                            format!("{}(", function.name)
                        }
                    };
                    // A name-only edit inside an existing call does not own
                    // the complete expression. New calls have their declared
                    // output type even before required arguments are entered.
                    if !existing_call && self.incompatible_function(function, expected) {
                        continue;
                    }
                    let label = format!(
                        "{}({})",
                        function.name,
                        function
                            .parameters
                            .iter()
                            .map(|p| format!(
                                "{}: {}{}{}",
                                p.0,
                                p.1,
                                if p.2 { "" } else { "[]" },
                                if p.3 { "?" } else { "" }
                            ))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    if !items.may_accept(&label, word_start) {
                        continue;
                    }
                    let return_detail = function
                        .definition
                        .as_ref()
                        .map(|definition| {
                            format!(
                                "return: {} (declared; {}); {}",
                                definition
                                    .return_type
                                    .as_ref()
                                    .map_or("unknown", ClassName::as_str),
                                if definition.return_type_is_single {
                                    "single"
                                } else {
                                    "plural"
                                },
                                function.detail
                            )
                        })
                        .unwrap_or_else(|| format!("return: unknown; {}", function.detail));
                    let mut item = self.item(
                        &label,
                        &insertion,
                        &return_detail,
                        &function.identity,
                        CompletionItemKind::FUNCTION,
                        TextRange::new(word_start, word_end),
                        use_snippets,
                    )?;
                    if existing_call {
                        let tail = &self.source[word_end..line.range.end];
                        let opening = tail.len() - tail.trim_start().len();
                        let end = word_end + opening + 1;
                        // Menu clients that only insert the prefix retain the
                        // ordinary edit below. Clients with an exact-edit adapter
                        // can own the existing opening delimiter in this item.
                        item.data = Some(serde_json::json!({
                            "nlaocs.lsp.completion-edit": {
                                "version": 1,
                                "edit": TextEdit {
                                    range: self.lines.range(&self.source, TextRange::new(word_start, end))?,
                                    new_text: format!("{}{}", function.name, &self.source[word_end..end]),
                                },
                            },
                        }));
                    }
                    items.push(item);
                }
            }
        }
        if variable_start.is_none() {
            let query = if expression {
                &self.source[word_start..offset]
            } else {
                prefix
            };
            let folded_query = query.to_lowercase();
            let start = if expression {
                word_start
            } else {
                line.code_start
            };
            for template in &self.index.templates {
                let allowed = if line.root {
                    matches!(template.kind, SyntaxKind::Structure | SyntaxKind::Event)
                } else if expression {
                    matches!(template.kind, SyntaxKind::Expression)
                } else {
                    matches!(
                        template.kind,
                        SyntaxKind::Effect | SyntaxKind::Section | SyntaxKind::Condition
                    )
                };
                if !allowed
                    || template.finite_capture && !finite_captures_current
                    || matches!(line.scope, Scope::Conditions)
                        && matches!(template.kind, SyntaxKind::Effect | SyntaxKind::Section)
                    || !template.folded.starts_with(&folded_query)
                {
                    continue;
                }
                if !items.may_accept(&preview(&template.parts), start) {
                    continue;
                }
                let (item, whole_template) =
                    self.template_item(template, query, start, word_end, line, snippets)?;
                if template.kind == SyntaxKind::Expression
                    && self.incompatible_template(
                        template,
                        expected,
                        line,
                        type_edit,
                        &item,
                        whole_template,
                        references,
                        &mut completion_type_budget,
                    )
                {
                    continue;
                }
                items.push(item);
            }
        }
        let (mut items, result_limited) = items.finish();
        let is_incomplete = result_limited || self.index.limited || frontier_limited || !references;
        for (index, item) in items.iter_mut().enumerate() {
            item.sort_text = Some(format!("{index:06}"));
        }
        // A provider can fail on the analysis thread while this menu is built.
        // Retry once without negative type filtering if its shared proof expired.
        if certified_types && self.index.typing.as_ref().is_some_and(|i| !i.current()) {
            return self.at_with_references(position, snippets, references);
        }
        Ok(CompletionList {
            is_incomplete,
            items,
        })
    }
    fn template_item(
        &self,
        template: &Template,
        query: &str,
        start: usize,
        word_end: usize,
        line: &Line,
        snippets: bool,
    ) -> Result<(CompletionItem, bool), String> {
        let header = matches!(
            template.kind,
            SyntaxKind::Event | SyntaxKind::Section | SyntaxKind::Structure
        );
        let end = word_end;
        let after = &self.source[end..line.range.end];
        let colon_end = (header && after.trim_start().starts_with(':'))
            .then(|| end + after.len() - after.trim_start().len() + 1);
        let suffix = !self.source[colon_end.unwrap_or(end)..line.range.end]
            .trim()
            .is_empty();
        let use_snippets = snippets && !suffix;
        let mut insertion = if suffix {
            // Preserve arguments and fixed words already present to the right.
            template
                .leading
                .split_whitespace()
                .take(query.split_whitespace().count().max(1))
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            render(&template.parts, use_snippets)
        };
        if colon_end.is_some() {
            // Keep the original colon outside the edit even when a plain-text
            // template stops before its first argument slot.
            insertion = insertion.trim_end_matches(':').trim_end().to_owned();
        }
        if self.source[end..line.range.end].starts_with(char::is_whitespace) {
            insertion = insertion.trim_end().to_owned();
        }
        let label = preview(&template.parts);
        let detail = if template.kind == SyntaxKind::Expression {
            format!(
                "{}; {}",
                self.index
                    .typing
                    .as_ref()
                    .map(|i| i.output_description(&template.identity))
                    .unwrap_or_else(|| "return: unknown; cardinality: unknown".into()),
                template.detail
            )
        } else {
            template.detail.clone()
        };
        let mut item = self.item(
            &label,
            &insertion,
            &detail,
            &template.identity,
            CompletionItemKind::SNIPPET,
            TextRange::new(start, end),
            use_snippets,
        )?;
        item.data = Some(
            serde_json::json!({"syntaxKind":format!("{:?}", template.kind),"syntaxIdentity":template.identity}),
        );
        if template.finite_capture {
            item.data.as_mut().unwrap()["finiteCaptureValue"] = serde_json::json!(true);
        }
        item.documentation = Some(super::hover_text::completion(
            &template.label,
            Some(&label),
            &detail,
            &template.identity,
            template.documentation.as_deref(),
            &self.index.documentation_format,
        ));
        // The definition owns the complete proposal only without a retained
        // right suffix; render(false) itself may stop at the first slot.
        Ok((item, !suffix))
    }
    fn item(
        &self,
        label: &str,
        insertion: &str,
        detail: &str,
        identity: &str,
        kind: CompletionItemKind,
        range: TextRange,
        snippets: bool,
    ) -> Result<CompletionItem, String> {
        Ok(CompletionItem {
            label: label.into(),
            kind: Some(kind),
            detail: Some(detail.into()),
            documentation: Some(Documentation::String(format!(
                "Syntax:\n{label}\n\n{detail}\nRegistration: {identity}"
            ))),
            filter_text: Some(insertion.split("${").next().unwrap_or(insertion).into()),
            insert_text_format: Some(if snippets {
                InsertTextFormat::SNIPPET
            } else {
                InsertTextFormat::PLAIN_TEXT
            }),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range: self.lines.range(&self.source, range)?,
                new_text: insertion.into(),
            })),
            ..CompletionItem::default()
        })
    }
}

fn raw_context(
    tree: &skript_parser::RawTree,
    source: &str,
    lines: &LineIndex,
    references: bool,
) -> Result<BTreeMap<u32, Line>, String> {
    let mut context = BTreeMap::new();
    let mut ambiguous = HashSet::new();
    for node in tree.iter() {
        let Some(range) = exact_native(&node.line.span) else {
            continue;
        };
        let Some(text) = source.get(range.start..range.end) else {
            continue;
        };
        let text = text.trim_end_matches('\n').trim_end_matches('\r');
        if text != node.line.raw_text || !text.starts_with(&node.line.indentation.text) {
            continue;
        }
        let row = lines.position(source, range.start)?.line;
        if context.contains_key(&row) {
            ambiguous.insert(row);
        }
        let mut ancestors = Vec::new();
        let mut editing_parent = node.parent;
        if node.kind == RawNodeKind::Blank {
            while let Some(parent) = editing_parent.and_then(|id| tree.get(id)) {
                if parent.line.indentation.text.len() < node.line.indentation.text.len() {
                    break;
                }
                editing_parent = parent.parent;
            }
        }
        let mut parent = if references { editing_parent } else { None };
        while let Some(id) = parent {
            if ancestors.len() >= tree.nodes.len() {
                return Err("cyclic completion RawTree".into());
            }
            let ancestor = tree.get(id).ok_or("missing completion parent")?;
            if exact_native(&ancestor.line.span).is_none() {
                ancestors.clear();
                break;
            }
            ancestors.push(id);
            parent = ancestor.parent;
        }
        context.insert(
            row,
            Line {
                node: Some(node.id),
                scope: if editing_parent.is_none() && node.line.indentation.text.is_empty() {
                    Scope::Statements
                } else if references {
                    Scope::Raw
                } else {
                    Scope::Pending
                },
                range: TextRange::new(range.start, range.start + text.len()),
                code_start: range.start + node.line.indentation.text.len(),
                parent: editing_parent,
                ancestors,
                root: editing_parent.is_none() && node.line.indentation.text.is_empty(),
                section: node.kind == RawNodeKind::Section,
                blocked: matches!(node.kind, RawNodeKind::Comment | RawNodeKind::Invalid),
                comments: node
                    .line
                    .trailing_trivia
                    .iter()
                    .filter(|t| {
                        matches!(
                            t.kind,
                            RawTriviaKind::LineComment | RawTriviaKind::BlockComment
                        )
                    })
                    .flat_map(|t| t.span.origins.iter().map(|o| o.original_range))
                    .collect(),
            },
        );
    }
    for row in ambiguous {
        context.remove(&row);
    }
    Ok(context)
}
fn starts(text: &str, prefix: &str) -> bool {
    text.to_lowercase().starts_with(&prefix.to_lowercase())
}
fn word(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-')
}
fn overlaps(a: TextRange, b: TextRange) -> bool {
    a.start < b.end && b.start < a.end || a.is_empty() && b.start <= a.start && a.start < b.end
}

#[derive(Clone, Copy)]
enum Lexical {
    Code(bool),
    String,
    Variable(usize),
    Comment,
}
fn lexical_context(text: &str) -> Lexical {
    let mut cursor = cursor::LexicalCursor::new(text);
    while cursor.next().is_some() {}
    cursor.state()
}

#[cfg(test)]
impl Document {
    pub fn test_fixture(source: Arc<str>) -> Self {
        let mapped = skript_parser::MappedSource::identity(Arc::clone(&source));
        let raw = skript_parser::parse_raw_tree(
            &mapped,
            skript_parser::RawTreeOptions::for_skript_version(2, 15),
        );
        let lines = LineIndex::new(&source);
        let context = raw
            .iter()
            .map(|node| {
                (
                    node.line.number as u32,
                    Line {
                        node: Some(node.id),
                        scope: Scope::Statements,
                        range: node.line.content_span.virtual_range,
                        code_start: node.line.content_span.virtual_range.start
                            + node.line.indentation.text.len(),
                        parent: node.parent,
                        ancestors: node.parent.into_iter().collect(),
                        root: node.parent.is_none() && node.line.indentation.text.is_empty(),
                        section: node.kind == RawNodeKind::Section,
                        blocked: false,
                        comments: Vec::new(),
                    },
                )
            })
            .collect();
        let label = source.to_string();
        Self {
            source,
            lines,
            context,
            entry_scopes: Vec::new(),
            variables: Vec::new(),
            parameters: HashMap::new(),
            functions: Vec::new(),
            index: Arc::new(CatalogIndex {
                templates: vec![Template {
                    finite_capture: false,
                    documentation: None,
                    kind: SyntaxKind::Structure,
                    label: label.clone(),
                    detail: "fixture".into(),
                    identity: "fixture".into(),
                    leading: label.clone(),
                    folded: label.to_lowercase(),
                    parts: vec![Part::Text(label)],
                }],
                functions: Vec::new(),
                limited: false,
                patterns: Vec::new(),
                raw_options: skript_parser::RawTreeOptions::for_skript_version(2, 15),
                documentation_format: lsp_types::MarkupKind::PlainText,
                typing: None,
            }),
            blocked: Vec::new(),
            function_namespace_complete: true,
            type_context_exact: false,
            accepted_slots: HashMap::new(),
            semantic_types: None,
        }
    }
}

#[cfg(test)]
impl CatalogIndex {
    pub fn test_syntax_index() -> Arc<Self> {
        Document::test_fixture(Arc::from("hold pattern")).index
    }
    pub fn test_signature_index() -> Arc<Self> {
        let mut index = Self::test_syntax_index();
        let label = "hold %objects% to %locations%";
        let rules = syntax_pattern_parser::syntax::PluralRules::from_json(include_str!(
            "../../syntax-pattern-parser/tests/data/PluralRules-2.15.4.json"
        ))
        .unwrap();
        let parsed = syntax_pattern_parser::syntax::parse(label, &rules).unwrap();
        Arc::get_mut(&mut index).unwrap().patterns.push(Pattern {
            documentation: None,
            kind: SyntaxKind::Structure,
            label: label.into(),
            detail: "fixture".into(),
            identity: "fixture".into(),
            graph: continuation::Graph::new(&parsed.elements).unwrap(),
        });
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_show_argument_differences_and_only_identical_edits_are_merged() {
        let grammar = "move %objects% (to|towards) %locations% [retaining %-objects%]";
        let rules = test_plural_rules();
        let parsed = syntax_pattern_parser::syntax::parse(grammar, &rules).unwrap();
        let mut document = with_functions("on load:\n    mo");
        let index = Arc::get_mut(&mut document.index).unwrap();
        for parts in variants(&parsed.elements, 0, &mut false, &rules) {
            let parts = normalize_parts(parts);
            index.templates.push(Template {
                finite_capture: false,
                documentation: Some(Arc::from("Supplied syntax description")),
                kind: SyntaxKind::Effect,
                label: grammar.into(),
                detail: "Effect / Move / Test".into(),
                identity: "move:0".into(),
                leading: "move ".into(),
                folded: "move ".into(),
                parts,
            });
        }
        assert_eq!(index.templates.len(), 4);
        index.templates.push(index.templates[0].clone());
        let mut other = index.templates[0].clone();
        other.identity = "other-move:0".into();
        index.templates.push(other);
        for snippets in [true, false] {
            let list = document.at(Position::new(1, 6), snippets).unwrap();
            assert_eq!(list.items.len(), 5);
            assert_eq!(
                list.items
                    .iter()
                    .map(|item| item.label.as_str())
                    .collect::<HashSet<_>>()
                    .len(),
                4
            );
            assert!(
                list.items
                    .iter()
                    .any(|item| item.label.contains("to %locations% retaining %-objects%"))
            );
            assert!(
                list.items
                    .iter()
                    .any(|item| item.label.contains("towards %locations%"))
            );
            for item in &list.items {
                assert!(!item.label.contains("${"));
                let Some(Documentation::String(text)) = &item.documentation else {
                    panic!("full docs")
                };
                assert!(text.contains(grammar));
                assert!(text.contains("Supplied syntax description"));
                assert!(text.contains("Expected types and addon guards are not checked."));
            }
        }
    }

    #[test]
    fn markdown_completion_keeps_identical_edits_and_registration_identity() {
        for source in ["on load:\n    mo", "on load:\n    move player t"] {
            let grammar = "move %objects% (to|towards) %locations%";
            let rules = test_plural_rules();
            let parsed = syntax_pattern_parser::syntax::parse(grammar, &rules).unwrap();
            let mut document = with_functions(source);
            let index = Arc::get_mut(&mut document.index).unwrap();
            for parts in variants(&parsed.elements, 0, &mut false, &rules) {
                let parts = normalize_parts(parts);
                index.templates.push(Template {
                    finite_capture: false,
                    documentation: None,
                    kind: SyntaxKind::Effect,
                    label: grammar.into(),
                    detail: "Effect / Move".into(),
                    identity: "move:0".into(),
                    leading: "move ".into(),
                    folded: "move ".into(),
                    parts,
                });
            }
            index.patterns.push(Pattern {
                documentation: None,
                kind: SyntaxKind::Effect,
                label: grammar.into(),
                detail: "Effect / Move".into(),
                identity: "move:0".into(),
                graph: continuation::Graph::new(&parsed.elements).unwrap(),
            });
            let position = Position::new(1, source.lines().nth(1).unwrap().len() as u32);
            let plain = document.at(position, true).unwrap();
            Arc::get_mut(&mut document.index)
                .unwrap()
                .documentation_format = lsp_types::MarkupKind::Markdown;
            let markdown = document.at(position, true).unwrap();
            assert!(!plain.items.is_empty());
            assert_eq!(plain.items.len(), markdown.items.len());
            for (plain, markdown) in plain.items.iter().zip(&markdown.items) {
                assert_eq!(plain.label, markdown.label);
                assert_eq!(plain.text_edit, markdown.text_edit);
                assert_eq!(plain.insert_text_format, markdown.insert_text_format);
                assert_eq!(plain.data, markdown.data);
                let Some(Documentation::MarkupContent(content)) = &markdown.documentation else {
                    panic!("negotiated markdown");
                };
                assert_eq!(content.kind, lsp_types::MarkupKind::Markdown);
                assert!(content.value.contains("skript-pattern"));
                assert!(content.value.contains(grammar));
            }
        }
    }

    #[test]
    fn equal_continuation_words_keep_distinct_syntax_provenance() {
        let grammar = "move %objects% to %locations%";
        let parsed = syntax_pattern_parser::syntax::parse(grammar, &test_plural_rules()).unwrap();
        let mut document = with_functions("on load:\n    move player t");
        let index = Arc::get_mut(&mut document.index).unwrap();
        for identity in ["first:0", "second:0", "first:0"] {
            index.patterns.push(Pattern {
                documentation: Some(Arc::from("Registered syntax description")),
                kind: SyntaxKind::Effect,
                label: grammar.into(),
                detail: identity.into(),
                identity: identity.into(),
                graph: continuation::Graph::new(&parsed.elements).unwrap(),
            });
        }
        let list = document.at(Position::new(1, 17), false).unwrap();
        assert_eq!(list.items.len(), 2);
        assert!(list.items.iter().all(|item| item.label == "to"));
        assert_eq!(
            list.items
                .iter()
                .map(|item| item.data.as_ref().unwrap()["syntaxIdentity"]
                    .as_str()
                    .unwrap())
                .collect::<HashSet<_>>()
                .len(),
            2
        );
    }

    fn test_plural_rules() -> syntax_pattern_parser::syntax::PluralRules {
        syntax_pattern_parser::syntax::PluralRules::from_json(include_str!(
            "../../syntax-pattern-parser/tests/data/PluralRules-2.15.4.json"
        ))
        .unwrap()
    }
    use syntax_pattern_parser::syntax::{Span, Spanned};

    #[test]
    fn nested_braces_keep_the_complete_variable_history_query_and_edit() {
        let source = "on join:\n    send {_outer{inner}} to console\n    send {_outer{in";
        let mut document = Document::test_fixture(Arc::from(source));
        document.variables.push(Variable {
            source: "{_outer{inner}}".into(),
            parent: document.context[&2].parent,
            local: true,
            observed: source.find("{_outer").unwrap(),
            canonical_name: None,
        });
        let list = document
            .at(
                document.lines.position(source, source.len()).unwrap(),
                false,
            )
            .unwrap();
        let item = list
            .items
            .iter()
            .find(|item| item.label == "{_outer{inner}}")
            .expect("whole native variable name remains visible");
        assert_eq!(edit(item).range.start, Position::new(2, 9));
        assert_eq!(edit(item).new_text, "{_outer{inner}}");
    }

    fn edit(item: &CompletionItem) -> &TextEdit {
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("expected an exact edit")
        };
        edit
    }
    fn function(name: &str, class: &str, returns_value: bool) -> Function {
        Function {
            name: name.into(),
            identity: format!("{name}:{class}:{returns_value}"),
            detail: "fixture".into(),
            parameters: vec![("argument".into(), class.into(), true, false)],
            returns_value,
            definition: None,
        }
    }
    fn with_functions(source: &str) -> Document {
        let mut document = Document::test_fixture(Arc::from(source));
        document.index = Arc::new(CatalogIndex {
            templates: Vec::new(),
            functions: vec![function("sin", "Number", true)],
            limited: false,
            patterns: Vec::new(),
            raw_options: skript_parser::RawTreeOptions::for_skript_version(2, 15),
            documentation_format: lsp_types::MarkupKind::PlainText,
            typing: None,
        });
        document
    }
    #[test]
    fn pattern_ast_templates_preserve_order_and_bound_choices_without_inserting_controls_or_regex()
    {
        let element = |value| Spanned::new(value, Span::new(0, 0));
        let elements = vec![
            element(PatternElement::ParseTag("tag".into())),
            element(PatternElement::ParseMark(7)),
            element(PatternElement::Literal("send ".into())),
            element(PatternElement::Choice(vec![
                vec![element(PatternElement::Literal("a ".into()))],
                vec![element(PatternElement::Literal("b ".into()))],
            ])),
            element(PatternElement::Regex("secret.*".into())),
            element(PatternElement::Literal(" $}\\".into())),
        ];
        let mut limited = false;
        let choices = variants(&elements, 0, &mut limited, &test_plural_rules());
        assert_eq!(choices.len(), 2);
        assert!(!limited);
        assert_eq!(render(&choices[0], false), "send a ");
        assert_eq!(render(&choices[1], false), "send b ");
        assert_eq!(render(&choices[0], true), "send a ${1:value} \\$\\}\\\\");
        let branches = (0..40)
            .map(|n| vec![element(PatternElement::Literal(format!("branch{n}")))])
            .collect();
        assert_eq!(
            variants(
                &[element(PatternElement::Choice(branches))],
                0,
                &mut limited,
                &test_plural_rules()
            )
            .len(),
            MAX_VARIANTS
        );
        assert!(limited);
        let optional_prefix = vec![
            element(PatternElement::Option(vec![element(
                PatternElement::Literal("the".into()),
            )])),
            element(PatternElement::Literal(" name".into())),
        ];
        let choices = variants(&optional_prefix, 0, &mut false, &test_plural_rules());
        assert_eq!(render(&normalize_parts(choices[0].clone()), false), "name");
        assert_eq!(
            render(&normalize_parts(choices[1].clone()), false),
            "the name"
        );
    }
    #[test]
    fn function_edits_preserve_existing_arguments_and_negotiate_snippets() {
        for (source, column, snippets, expected) in [
            ("on load:\r\n    send si", 11, true, "sin(${1:argument})"),
            ("on load:\r\n    send si", 11, false, "sin("),
            ("on load:\r\n    send si(1)", 11, true, "sin"),
        ] {
            let view = with_functions(source);
            let list = view.at(Position::new(1, column), snippets).unwrap();
            assert_eq!(list.items.len(), 1);
            assert_eq!(edit(&list.items[0]).new_text, expected);
            assert_eq!(
                edit(&list.items[0]).range,
                lsp_types::Range::new(Position::new(1, 9), Position::new(1, 11))
            );
        }
        let view = with_functions("on load:\n    send \"日本😀\"\n");
        assert!(
            view.at(Position::new(1, 13), true).is_err(),
            "surrogate interiors are invalid"
        );
        let view = with_functions("on load:\n    send sxx(1)");
        assert!(
            view.at(Position::new(1, 10), true)
                .unwrap()
                .items
                .is_empty(),
            "a different right suffix is omitted instead of duplicated"
        );
    }
    #[test]
    fn existing_call_items_offer_exact_delimiter_ownership_without_changing_prefix_edits() {
        for (line, column, fallback, complete, end, expected) in [
            ("    send si(1)", 11, "sin", "sin(", 12, "    send sin(1)"),
            ("    send sin(1)", 11, "si", "sin(", 13, "    send sin(1)"),
            (
                "    send si  (1)",
                11,
                "sin",
                "sin  (",
                14,
                "    send sin  (1)",
            ),
        ] {
            let source = format!("on load:\n{line}");
            let view = with_functions(&source);
            let list = view.at(Position::new(1, column), true).unwrap();
            let item = &list.items[0];
            assert_eq!(item.kind, Some(CompletionItemKind::FUNCTION));
            assert_eq!(item.insert_text_format, Some(InsertTextFormat::PLAIN_TEXT));
            assert_eq!(edit(item).new_text, fallback);
            assert_eq!(edit(item).range.end, Position::new(1, column));
            let policy = &item.data.as_ref().unwrap()["nlaocs.lsp.completion-edit"];
            assert_eq!(policy["version"], 1);
            let exact: TextEdit = serde_json::from_value(policy["edit"].clone()).unwrap();
            assert_eq!(exact.new_text, complete);
            assert_eq!(exact.range.start, Position::new(1, 9));
            assert_eq!(exact.range.end, Position::new(1, end));
            assert_eq!(
                format!("{}{}{}", &line[..9], exact.new_text, &line[end as usize..]),
                expected
            );
        }
        let view = with_functions("on load:\n    send si");
        let list = view.at(Position::new(1, 11), true).unwrap();
        let item = &list.items[0];
        assert!(item.data.is_none());
        assert_eq!(item.insert_text_format, Some(InsertTextFormat::SNIPPET));
        assert_eq!(edit(item).new_text, "sin(${1:argument})");
    }
    #[test]
    fn void_document_shadow_does_not_resurrect_a_catalog_function_and_overloads_survive() {
        let mut view = with_functions("on load:\n    send si");
        view.functions = vec![
            function("sin", "Number", false),
            function("sin", "String", true),
        ];
        let list = view.at(Position::new(1, 11), false).unwrap();
        assert_eq!(list.items.len(), 1);
        assert!(list.items[0].label.contains("String"));
        view.functions[0].returns_value = true;
        assert_eq!(view.at(Position::new(1, 11), false).unwrap().items.len(), 2);
    }
    #[test]
    fn local_history_stays_in_its_raw_body_and_preserves_the_existing_closing_brace() {
        let source = "on load:\n    send {_mo}\n    send {_money}\n    send {_mo} to console\non join:\n    send {_mo}\n";
        let mut view = with_functions(source);
        view.variables.push(Variable {
            source: "{_money}".into(),
            parent: view.context[&2].parent,
            local: true,
            observed: source.find("{_money}").unwrap(),
            canonical_name: None,
        });
        let list = view.at(Position::new(3, 13), false).unwrap();
        assert_eq!(list.items.len(), 1);
        assert_eq!(edit(&list.items[0]).new_text, "{_money");
        assert_eq!(edit(&list.items[0]).range.end, Position::new(3, 13));
        assert!(
            view.at(Position::new(5, 13), false)
                .unwrap()
                .items
                .is_empty()
        );
        assert!(
            view.at(Position::new(1, 13), false)
                .unwrap()
                .items
                .is_empty(),
            "later references are not declarations"
        );
    }
    #[test]
    fn strings_comments_and_nested_interpolation_have_distinct_lexical_contexts() {
        assert!(matches!(
            lexical_context("send \"literal {_x"),
            Lexical::String
        ));
        assert!(matches!(
            lexical_context("send \"100%% {_x"),
            Lexical::String
        ));
        assert!(matches!(lexical_context("send \"%si"), Lexical::Code(true)));
        assert!(matches!(
            lexical_context("send {_outer%{_inner%si"),
            Lexical::Code(true)
        ));
        assert!(matches!(lexical_context("send 1 # {_x"), Lexical::Comment));
    }
    #[test]
    fn existing_fixed_suffix_is_not_inserted_twice() {
        let mut view = with_functions("on load:\n    send name of player");
        view.index = Arc::new(CatalogIndex {
            templates: vec![Template {
                finite_capture: false,
                documentation: None,
                kind: SyntaxKind::Expression,
                label: "name of objects".into(),
                detail: "fixture".into(),
                identity: "fixture".into(),
                leading: "name of ".into(),
                folded: "name of ".into(),
                parts: vec![Part::Text("name of ".into()), Part::Slot("objects".into())],
            }],
            functions: Vec::new(),
            limited: false,
            patterns: Vec::new(),
            raw_options: skript_parser::RawTreeOptions::for_skript_version(2, 15),
            documentation_format: lsp_types::MarkupKind::PlainText,
            typing: None,
        });
        let result = view.at(Position::new(1, 11), true).unwrap();
        assert_eq!(result.items.len(), 1);
        assert_eq!(edit(&result.items[0]).new_text, "na");
        assert_eq!(
            edit(&result.items[0]).range,
            lsp_types::Range::new(Position::new(1, 9), Position::new(1, 11))
        );
        assert_eq!(
            result.items[0].insert_text_format,
            Some(InsertTextFormat::PLAIN_TEXT)
        );
    }
    #[test]
    fn fixed_statement_words_survive_whitespace_before_expression_suggestions() {
        let mut view = with_functions("on load:\n    cancel ");
        let mut index = Arc::try_unwrap(view.index).ok().unwrap();
        index.templates.push(Template {
            finite_capture: false,
            documentation: None,
            kind: SyntaxKind::Effect,
            label: "cancel event".into(),
            detail: "fixture".into(),
            identity: "cancel".into(),
            leading: "cancel event".into(),
            folded: "cancel event".into(),
            parts: vec![Part::Text("cancel event".into())],
        });
        view.index = Arc::new(index);
        let list = view.at(Position::new(1, 11), true).unwrap();
        assert_eq!(list.items[0].label, "cancel event");
        assert_eq!(edit(&list.items[0]).new_text, "cancel event");
        assert_eq!(edit(&list.items[0]).range.start, Position::new(1, 4));
        assert!(list.items.iter().any(|item| item.label.starts_with("sin(")));
    }
    #[test]
    fn an_ambiguous_second_word_keeps_functions_and_fixed_statement_candidates() {
        let mut view = with_functions("on load:\n    send s");
        let mut index = Arc::try_unwrap(view.index).ok().unwrap();
        index.templates.push(Template {
            finite_capture: false,
            documentation: None,
            kind: SyntaxKind::Effect,
            label: "send subtitle".into(),
            detail: "fixture".into(),
            identity: "subtitle".into(),
            leading: "send subtitle ".into(),
            folded: "send subtitle ".into(),
            parts: vec![
                Part::Text("send subtitle ".into()),
                Part::Slot("object".into()),
            ],
        });
        view.index = Arc::new(index);
        let list = view.at(Position::new(1, 10), true).unwrap();
        assert_eq!(list.items[0].label, "send subtitle %object%");
        assert!(list.items.iter().any(|item| item.label.starts_with("sin(")));
    }
    #[test]
    fn existing_header_colon_and_comment_survive_a_partial_template_edit() {
        for (source, row, column, kind, leading, snippets, expected) in [
            (
                "on load:\n    loop: # keep",
                1,
                8,
                SyntaxKind::Section,
                "loop ",
                true,
                "loop",
            ),
            (
                "on load: # keep",
                0,
                5,
                SyntaxKind::Event,
                "on load:",
                true,
                "on load",
            ),
            (
                "on load:\n    loop:",
                1,
                8,
                SyntaxKind::Section,
                "loop ",
                false,
                "loop",
            ),
        ] {
            let view = with_functions(source);
            let template = Template {
                finite_capture: false,
                documentation: None,
                kind,
                label: "fixture".into(),
                detail: "fixture".into(),
                identity: "fixture".into(),
                leading: leading.into(),
                folded: leading.into(),
                parts: vec![
                    Part::Text(leading.into()),
                    Part::Slot("objects".into()),
                    Part::Text(":".into()),
                ],
            };
            let line = &view.context[&row];
            let cursor = view
                .lines
                .offset(&view.source, Position::new(row, column))
                .unwrap();
            let word_end = source[cursor..line.range.end].find(':').unwrap() + cursor;
            let item = view
                .template_item(
                    &template,
                    &source[line.code_start..cursor],
                    line.code_start,
                    word_end,
                    line,
                    snippets,
                )
                .unwrap()
                .0;
            assert_eq!(edit(&item).new_text, expected);
            assert_eq!(
                edit(&item).range.end,
                Position::new(row, if row == 1 { 8 } else { 7 })
            );
            let range = edit(&item).range;
            let start = view.lines.offset(&view.source, range.start).unwrap();
            let end = view.lines.offset(&view.source, range.end).unwrap();
            assert_eq!(
                format!(
                    "{}{}{}",
                    &source[..start],
                    edit(&item).new_text,
                    &source[end..]
                ),
                source
            );
        }
    }
    #[test]
    fn result_limit_keeps_registration_order_and_reports_incompleteness() {
        let mut view = with_functions("on load:\n    send ");
        view.index = Arc::new(CatalogIndex {
            templates: Vec::new(),
            functions: (0..MAX_ITEMS + 50)
                .map(|n| function(&format!("f{n:03}"), "Number", true))
                .collect(),
            limited: false,
            patterns: Vec::new(),
            raw_options: skript_parser::RawTreeOptions::for_skript_version(2, 15),
            documentation_format: lsp_types::MarkupKind::PlainText,
            typing: None,
        });
        let list = view.at(Position::new(1, 9), false).unwrap();
        assert!(list.is_incomplete);
        assert_eq!(list.items.len(), MAX_ITEMS);
        assert!(list.items[0].label.starts_with("f000("));
        assert!(
            list.items[MAX_ITEMS - 1]
                .label
                .starts_with(&format!("f{:03}(", MAX_ITEMS - 1))
        );
    }
    #[test]
    fn continuations_preserve_arguments_suffixes_and_the_full_grammar() {
        let grammar = "teleport %entities% (to|towards) %location%";
        for (marked, expected) in [
            ("teleport player t|", "teleport player to"),
            ("teleport name of player t|", "teleport name of player to"),
            (
                "teleport location(1, 2, 3) t|",
                "teleport location(1, 2, 3) to",
            ),
            (
                "teleport {_日本😀} t|o location(1, 2, 3)",
                "teleport {_日本😀} to location(1, 2, 3)",
            ),
        ] {
            let source = format!("on join:\n    {}", marked.replace('|', ""));
            let byte = 4 + marked.find('|').unwrap();
            let position = Position::new(
                1,
                source.lines().nth(1).unwrap()[..byte]
                    .encode_utf16()
                    .count() as u32,
            );
            let mut view = with_functions(&source);
            let mut index = Arc::try_unwrap(view.index).ok().unwrap();
            let rules = syntax_pattern_parser::syntax::PluralRules::from_json(include_str!(
                "../../syntax-pattern-parser/tests/data/PluralRules-2.15.4.json"
            ))
            .unwrap();
            let parsed = syntax_pattern_parser::syntax::parse(grammar, &rules).unwrap();
            index.patterns.push(Pattern {
                documentation: None,
                kind: SyntaxKind::Effect,
                label: grammar.into(),
                detail: "Effect / fixture".into(),
                identity: "effect:fixture:0".into(),
                graph: continuation::Graph::new(&parsed.elements).unwrap(),
            });
            view.index = Arc::new(index);
            let list = view.at(position, true).unwrap();
            let item = list
                .items
                .iter()
                .find(|i| i.label == "to")
                .expect("next grammar word");
            assert_eq!(list.items[0].label, "to");
            assert_eq!(item.kind, Some(CompletionItemKind::KEYWORD));
            let Documentation::String(documentation) = item.documentation.as_ref().unwrap() else {
                panic!("full grammar documentation");
            };
            assert!(documentation.contains(grammar));
            let edit = edit(item);
            let start = view.lines.offset(&source, edit.range.start).unwrap();
            let end = view.lines.offset(&source, edit.range.end).unwrap();
            assert_eq!(
                format!("{}{}{}", &source[..start], edit.new_text, &source[end..]),
                format!("on join:\n    {expected}")
            );
            assert_eq!(edit.range.end, position);
        }
    }

    #[test]
    fn suffix_filter_precedes_independent_completion_budgets() {
        let grammar = "move %objects% (to|towards) %location%";
        let rules = syntax_pattern_parser::syntax::PluralRules::from_json(include_str!(
            "../../syntax-pattern-parser/tests/data/PluralRules-2.15.4.json"
        ))
        .unwrap();
        let parsed = syntax_pattern_parser::syntax::parse(grammar, &rules).unwrap();
        for (suffix, expected_keywords, expected_functions) in [("o", 1, 12), ("x", 0, 12)] {
            let source = format!("on load:\n    move player t{suffix}");
            let mut view = with_functions(&source);
            let index = Arc::get_mut(&mut view.index).unwrap();
            index.patterns.push(Pattern {
                documentation: None,
                kind: SyntaxKind::Effect,
                label: grammar.into(),
                detail: "fixture".into(),
                identity: "move:0".into(),
                graph: continuation::Graph::new(&parsed.elements).unwrap(),
            });
            index.functions = (0..12)
                .map(|n| function(&format!("t{n}{suffix}"), "Number", true))
                .collect();
            // Existing calls suppress argument snippets, making the suffix exact.
            let mut source = source;
            source.push_str("(1)");
            view.source = Arc::from(source.as_str());
            view.lines = LineIndex::new(&source);
            let line = view.context.get_mut(&1).unwrap();
            line.range.end = source.len();
            let list = view.at(Position::new(1, 17), true).unwrap();
            assert_eq!(
                list.items
                    .iter()
                    .filter(|i| i.kind == Some(CompletionItemKind::KEYWORD))
                    .count(),
                expected_keywords
            );
            assert_eq!(
                list.items
                    .iter()
                    .filter(|i| i.kind == Some(CompletionItemKind::FUNCTION))
                    .count(),
                expected_functions
            );
        }
    }

    #[test]
    fn discarded_continuations_do_not_consume_the_normal_candidate_budget() {
        let source = "on load:\n    move player tx(1)";
        let mut view = with_functions(source);
        let index = Arc::get_mut(&mut view.index).unwrap();
        let grammar = format!(
            "move %objects% ({})",
            (0..110)
                .map(|n| format!("to{n}"))
                .collect::<Vec<_>>()
                .join("|")
        );
        let rules = syntax_pattern_parser::syntax::PluralRules::from_json(include_str!(
            "../../syntax-pattern-parser/tests/data/PluralRules-2.15.4.json"
        ))
        .unwrap();
        let parsed = syntax_pattern_parser::syntax::parse(&grammar, &rules).unwrap();
        index.patterns.push(Pattern {
            documentation: None,
            kind: SyntaxKind::Effect,
            label: grammar,
            detail: "fixture".into(),
            identity: "move:0".into(),
            graph: continuation::Graph::new(&parsed.elements).unwrap(),
        });
        index.functions = vec![function("tx", "Number", true)];
        let list = view.at(Position::new(1, 17), true).unwrap();
        assert_eq!(list.items.len(), 1);
        assert!(list.items[0].label.starts_with("tx("));
        assert_eq!(edit(&list.items[0]).new_text, "t");
    }
}

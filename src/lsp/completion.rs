//! Bounded pattern continuations, input templates and current-revision history.
//!
//! These are editing suggestions, not successful parse candidates. Expected
//! types, execution scopes and addon guards are deliberately not reconstructed.
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
use syntaxes::{Catalog, Syntax, SyntaxKind};

mod continuation;
mod cursor;
mod signature;
pub(super) use signature::{SignatureCapabilities, SignatureQuery};

const MAX_ITEMS: usize = 100;
const MAX_VARIANTS: usize = 16;
const MAX_TEMPLATE_BYTES: usize = 1024;
const MAX_CONTEXT_ALTERNATIVES: usize = 8;
const MAX_SYNTAX_SOURCE_BYTES: usize = 256 * 1024;
const MAX_CONTINUATION_BYTES: usize = 4096;

#[derive(Clone)]
enum Part {
    Text(String),
    Slot(String),
}
#[derive(Clone)]
struct Template {
    kind: SyntaxKind,
    label: String,
    detail: String,
    identity: String,
    leading: String,
    folded: String,
    parts: Vec<Part>,
}
struct Pattern {
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
}
impl Function {
    fn shape(&self) -> Vec<(&str, bool)> {
        self.parameters
            .iter()
            .map(|(_, class, single, _)| (class.as_str(), *single))
            .collect()
    }
    fn from_definition(definition: &FunctionDefinition) -> Self {
        Self {
            name: definition.name.clone(),
            identity: definition.registration_id.clone(),
            detail: "Function / current document; argument types are not inferred".into(),
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
        }
    }
}

pub(super) struct CatalogIndex {
    templates: Vec<Template>,
    patterns: Vec<Pattern>,
    functions: Vec<Function>,
    limited: bool,
    raw_options: skript_parser::RawTreeOptions,
}
impl CatalogIndex {
    pub fn new(catalog: &Catalog) -> Self {
        let mut index = Self {
            templates: Vec::new(),
            patterns: Vec::new(),
            functions: Vec::new(),
            limited: false,
            raw_options: raw_options(catalog),
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
            for (pattern_index, pattern) in common.patterns.iter().enumerate() {
                if pattern.source.len() > MAX_TEMPLATE_BYTES {
                    index.limited = true;
                    continue;
                }
                let detail = format!(
                    "{:?} / {}; types and addon guards are not checked",
                    syntax.kind(),
                    common.addon.name
                );
                let identity = format!("{}:{pattern_index}", common.registration_id.as_str());
                let event_prefix = matches!(syntax, Syntax::Event(event) if event.has_on_prefix);
                if let Some(graph) = continuation::Graph::with_prefix(
                    if event_prefix { "on " } else { "" },
                    &pattern.parsed.elements,
                ) {
                    index.patterns.push(Pattern {
                        kind: syntax.kind(),
                        label: pattern.source.clone(),
                        detail: detail.clone(),
                        identity: identity.clone(),
                        graph,
                    });
                } else {
                    index.limited = true;
                }
                let variants = variants(&pattern.parsed.elements, 0, &mut index.limited);
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

fn raw_options(catalog: &Catalog) -> skript_parser::RawTreeOptions {
    let version = catalog
        .source()
        .and_then(|source| source.runtime.as_ref())
        .and_then(|runtime| {
            runtime
                .plugins
                .iter()
                .find(|plugin| plugin.name.eq_ignore_ascii_case("Skript"))
        })
        .map(|plugin| plugin.version.as_str());
    let mut numbers = version
        .unwrap_or("2.9")
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u32>().ok());
    let (major, minor) = match (numbers.next(), numbers.next()) {
        (Some(major), Some(minor)) => (major, minor),
        _ => (2, 9),
    };
    skript_parser::RawTreeOptions::for_skript_version(major, minor)
}

fn variants(
    elements: &[SpannedPatternElement],
    depth: usize,
    limited: &mut bool,
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
                types
                    .alternatives
                    .iter()
                    .map(|t| t.name.as_str())
                    .collect::<Vec<_>>()
                    .join("/"),
            )]],
            PatternElement::Regex(_) => vec![vec![Part::Slot("value".into())]],
            PatternElement::Group(group) => variants(group, depth + 1, limited),
            PatternElement::Choice(branches) => {
                let mut choices = Vec::new();
                for branch in branches {
                    choices.extend(variants(branch, depth + 1, limited));
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
                choices.extend(variants(option, depth + 1, limited));
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

struct Line {
    range: TextRange,
    code_start: usize,
    parent: Option<RawNodeId>,
    ancestors: Vec<RawNodeId>,
    root: bool,
    blocked: bool,
    comments: Vec<TextRange>,
}
struct Variable {
    source: String,
    parent: Option<RawNodeId>,
    local: bool,
    observed: usize,
}
pub(super) struct Document {
    source: Arc<str>,
    lines: LineIndex,
    context: BTreeMap<u32, Line>,
    variables: Vec<Variable>,
    functions: Vec<Function>,
    index: Arc<CatalogIndex>,
    blocked: Vec<TextRange>,
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
            variables: Vec::new(),
            functions: Vec::new(),
            index,
            blocked: Vec::new(),
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
        line.blocked
            || line
                .comments
                .iter()
                .any(|r| r.start <= offset && offset <= r.end)
            || matches!(
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
            .fault_regions
            .iter()
            .flat_map(|r| r.span.origins.iter())
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
        let context = raw_context(&result.raw_tree, &source, &lines, true)?;
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
        let mut variables = Vec::new();
        for id in reachable {
            let node = nodes[&id];
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
            });
        }
        variables.sort_by_key(|v| v.observed);
        let mut functions = Vec::new();
        let mut names = HashSet::new();
        for registration in result.functions.registrations() {
            if names.insert(registration.definition.name.as_str()) {
                functions.extend(
                    result
                        .functions
                        .lookup_references(&registration.definition.name, FunctionScope::Local)
                        .into_iter()
                        .map(|r| Function::from_definition(&r.definition)),
                );
            }
        }
        Ok(Self {
            source,
            lines,
            context,
            variables,
            functions,
            index,
            blocked,
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
                range: TextRange::new(start, start + text.len()),
                code_start: start + text.len(),
                parent: None,
                ancestors: Vec::new(),
                root: text.is_empty(),
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
        let prefix = &self.source[line.code_start..offset];
        let folded_prefix = prefix.to_lowercase();
        let state = lexical_context(prefix);
        if matches!(state, Lexical::String | Lexical::Comment) {
            return Ok(empty());
        }
        let mut items = Vec::new();
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
        if variable_start.is_some() && !references {
            return Ok(empty());
        }
        let mut frontier_limited = prefix.len() > MAX_CONTINUATION_BYTES;
        if variable_start.is_none() && word_start > line.code_start && !frontier_limited {
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
                if !allowed {
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
            let mut seen = HashSet::new();
            for (fixed, slots, _, keyword, pattern) in continuations {
                if !seen.insert(keyword.clone()) {
                    continue;
                }
                let detail = format!(
                    "{}; continuation: {fixed} fixed characters, {slots} structural argument(s)",
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
                    serde_json::json!({"syntaxKind":format!("{:?}", pattern.kind),"continuation":true}),
                );
                item.documentation = Some(Documentation::String(format!(
                    "Syntax:\n{}\n\n{detail}\nRegistration: {}",
                    pattern.label, pattern.identity
                )));
                items.push(item);
            }
        }
        // Continuations have a separate generation budget: they may all fail
        // the right-suffix check, so they must not consume the fallback budget.
        let generated_continuations = items.len();
        let expression = variable_start.is_some()
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
        // Whitespace may start an argument while still matching fixed words in
        // the current statement. Keep those matches ahead of generic expressions.
        if expression && !line.root && variable_start.is_none() {
            for template in &self.index.templates {
                if items.len().saturating_sub(generated_continuations) > MAX_ITEMS {
                    break;
                }
                if matches!(
                    template.kind,
                    SyntaxKind::Effect | SyntaxKind::Section | SyntaxKind::Condition
                ) && template.folded.starts_with(&folded_prefix)
                {
                    items.push(self.template_item(
                        template,
                        prefix,
                        line.code_start,
                        word_end,
                        line,
                        snippets,
                    )?);
                }
            }
        }
        if expression {
            let start = variable_start.unwrap_or(word_start);
            let mut end = word_end;
            if variable_start.is_some() {
                let suffix = &self.source[offset..line.range.end];
                if let Some(close) = suffix.find('}') {
                    if !suffix[..close].contains(['{', '%']) {
                        end = offset + close + 1;
                    }
                }
            }
            let query = &self.source[start..offset];
            let mut seen = HashSet::new();
            for variable in &self.variables {
                if items.len().saturating_sub(generated_continuations) > MAX_ITEMS {
                    break;
                }
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
                if starts(search, query) && seen.insert(variable.source.as_str()) {
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
            if variable_start.is_none() && references {
                let query = &self.source[word_start..offset];
                for (index, function) in self
                    .functions
                    .iter()
                    .chain(&self.index.functions)
                    .enumerate()
                {
                    if items.len().saturating_sub(generated_continuations) > MAX_ITEMS {
                        break;
                    }
                    if !function.returns_value || !starts(&function.name, query) {
                        continue;
                    }
                    if index >= self.functions.len()
                        && self
                            .functions
                            .iter()
                            .any(|f| f.name == function.name && f.shape() == function.shape())
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
                    items.push(self.item(
                        &label,
                        &insertion,
                        &function.detail,
                        &function.identity,
                        CompletionItemKind::FUNCTION,
                        TextRange::new(word_start, word_end),
                        use_snippets,
                    )?);
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
                if items.len().saturating_sub(generated_continuations) > MAX_ITEMS {
                    break;
                }
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
                if !allowed || !template.folded.starts_with(&folded_query) {
                    continue;
                }
                items.push(self.template_item(template, query, start, word_end, line, snippets)?);
            }
        }
        let mut seen = HashSet::new();
        items.retain(|item| {
            let identity =
                item.documentation
                    .as_ref()
                    .and_then(|documentation| match documentation {
                        Documentation::String(text) => {
                            text.rsplit_once("Registration: ").map(|(_, id)| id)
                        }
                        _ => None,
                    });
            if item.kind == Some(CompletionItemKind::SNIPPET) {
                let leading = item.filter_text.as_deref().unwrap_or("").trim_end();
                seen.insert((identity.unwrap_or("").to_owned(), leading.to_owned()))
            } else {
                true
            }
        });
        // Some menu clients insert only up to the cursor. Keep edits in that
        // prefix when an existing right suffix already matches the suggestion.
        // Omit ambiguous middle replacements instead of duplicating the suffix.
        items.retain_mut(|item| {
            let Some(CompletionTextEdit::Edit(edit)) = &mut item.text_edit else {
                return false;
            };
            let Some(end) = self.lines.offset(&self.source, edit.range.end) else {
                return false;
            };
            if end <= offset {
                return true;
            }
            let suffix = &self.source[offset..end];
            let suffix = if item.insert_text_format == Some(InsertTextFormat::SNIPPET) {
                escape(suffix)
            } else {
                suffix.to_owned()
            };
            let Some(prefix) = edit.new_text.strip_suffix(&suffix) else {
                return false;
            };
            edit.new_text = prefix.to_owned();
            edit.range.end = position;
            true
        });
        let continuation_count = items
            .iter()
            .filter(|item| {
                item.data
                    .as_ref()
                    .is_some_and(|data| data["continuation"] == true)
            })
            .count();
        let alternatives = if continuation_count > 0 {
            MAX_CONTEXT_ALTERNATIVES
        } else {
            MAX_ITEMS
        };
        let before = items.len();
        let mut ordinary = 0;
        items.retain(|item| {
            if item
                .data
                .as_ref()
                .is_some_and(|data| data["continuation"] == true)
            {
                return true;
            }
            ordinary += 1;
            ordinary <= alternatives
        });
        let is_incomplete = before > items.len()
            || items.len() > MAX_ITEMS
            || self.index.limited
            || frontier_limited
            || !references;
        items.truncate(MAX_ITEMS);
        for (index, item) in items.iter_mut().enumerate() {
            item.sort_text = Some(format!("{index:06}"));
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
    ) -> Result<CompletionItem, String> {
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
        let mut item = self.item(
            &template.label,
            &insertion,
            &template.detail,
            &template.identity,
            CompletionItemKind::SNIPPET,
            TextRange::new(start, end),
            use_snippets,
        )?;
        item.data = Some(serde_json::json!({"syntaxKind":format!("{:?}", template.kind)}));
        Ok(item)
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
        let mut parent = if references { node.parent } else { None };
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
                range: TextRange::new(range.start, range.start + text.len()),
                code_start: range.start + node.line.indentation.text.len(),
                parent: node.parent,
                ancestors,
                root: node.parent.is_none() && node.line.indentation.text.is_empty(),
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
                        range: node.line.content_span.virtual_range,
                        code_start: node.line.content_span.virtual_range.start
                            + node.line.indentation.text.len(),
                        parent: node.parent,
                        ancestors: node.parent.into_iter().collect(),
                        root: node.parent.is_none() && node.line.indentation.text.is_empty(),
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
            variables: Vec::new(),
            functions: Vec::new(),
            index: Arc::new(CatalogIndex {
                templates: vec![Template {
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
            }),
            blocked: Vec::new(),
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
        let choices = variants(&elements, 0, &mut limited);
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
                &mut limited
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
        let choices = variants(&optional_prefix, 0, &mut false);
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
        assert_eq!(list.items[0].label, "send subtitle");
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
                .unwrap();
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
            functions: (0..150)
                .map(|n| function(&format!("f{n:03}"), "Number", true))
                .collect(),
            limited: false,
            patterns: Vec::new(),
            raw_options: skript_parser::RawTreeOptions::for_skript_version(2, 15),
        });
        let list = view.at(Position::new(1, 9), false).unwrap();
        assert!(list.is_incomplete);
        assert_eq!(list.items.len(), 100);
        assert!(list.items[0].label.starts_with("f000("));
        assert!(list.items[99].label.starts_with("f099("));
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
    fn right_suffix_filter_precedes_the_context_alternative_limit() {
        let grammar = "move %objects% (to|towards) %location%";
        let rules = syntax_pattern_parser::syntax::PluralRules::from_json(include_str!(
            "../../syntax-pattern-parser/tests/data/PluralRules-2.15.4.json"
        ))
        .unwrap();
        let parsed = syntax_pattern_parser::syntax::parse(grammar, &rules).unwrap();
        for (suffix, expected_keywords, expected_functions) in [("o", 1, 8), ("x", 0, 12)] {
            let source = format!("on load:\n    move player t{suffix}");
            let mut view = with_functions(&source);
            let index = Arc::get_mut(&mut view.index).unwrap();
            index.patterns.push(Pattern {
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

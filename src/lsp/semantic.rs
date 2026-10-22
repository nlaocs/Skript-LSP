//! Revision-local semantic facts. Provider contracts supply meaning; native AST
//! captures and exact source projections supply geometry. No source grammar is
//! re-parsed here, and no runtime value or old document type is assumed.
use super::{
    positions::LineIndex,
    public_data::{VariableIdentity as IdentityData, parameter_bindings},
};
use lsp_types::{Location, Position, Range, Uri};
use parser_wasm::{
    DocumentParseResult,
    host::{AstNode, AstTree, CaptureValue, MappedSpan, SyntaxKind, source_geometry},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use skript_parser::{FunctionDefinition, FunctionScope, RawNodeKind, TextRange};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    sync::Arc,
};
use syntaxes::Catalog;
mod call_hierarchy;
mod document_symbols;
mod navigation;

const CHANGE: &str = "skript.semantic.variable-change";
const FLOW: &str = "skript.semantic.control-flow";
const IDENTITY: &str = "nlaocs.skript.variable-identity";
const PARAMETERS: &str = "skript.semantic.parameters";
const LOCAL_EFFECTS: &str = "skript.semantic.local-effects";
const EVALUATION_CONTEXT: &str = "skript.semantic.evaluation-context";
const OPERANDS: &str = "skript.semantic.operand-evaluation";
const VALUE_RESULT: &str = "skript.semantic.value-result";
const VARIABLE_READ: &str = "skript.semantic.variable-read";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum InferredType {
    Unknown,
    Values {
        classes: BTreeSet<String>,
        multiple: bool,
        maybe_unset: bool,
    },
}
impl InferredType {
    fn unset() -> Self {
        Self::Values {
            classes: BTreeSet::new(),
            multiple: false,
            maybe_unset: true,
        }
    }
    fn join(&self, other: &Self) -> Self {
        match (self, other) {
            (
                Self::Values {
                    classes: left,
                    multiple: lm,
                    maybe_unset: lu,
                },
                Self::Values {
                    classes: right,
                    multiple: rm,
                    maybe_unset: ru,
                },
            ) => Self::Values {
                classes: left.union(right).cloned().collect(),
                multiple: *lm || *rm,
                maybe_unset: *lu || *ru,
            },
            _ => Self::Unknown,
        }
    }
    pub fn description(&self, catalog: &Catalog) -> String {
        match self {
            Self::Unknown => "unknown (runtime value is not established here)".into(),
            Self::Values {
                classes,
                multiple,
                maybe_unset,
            } => {
                let mut names = classes
                    .iter()
                    .map(|class| {
                        catalog
                            .type_by_class_name(class)
                            .map_or_else(|| class.clone(), |ty| ty.code_name.as_str().to_owned())
                    })
                    .collect::<Vec<_>>();
                if *maybe_unset {
                    names.push("unset".into());
                }
                let description = names.join(" | ");
                if *multiple {
                    format!("{description} (list elements)")
                } else {
                    description
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Dependency {
    Function {
        definition: String,
        registration: String,
    },
    FunctionName(String),
    GlobalVariable(String),
    UnknownVariableAlias,
}

#[derive(Clone, Debug)]
pub(super) struct FunctionSymbol {
    pub definition: FunctionDefinition,
    pub range: Range,
    pub local: bool,
    pub name_range: Option<Range>,
    body: Option<call_hierarchy::FunctionBody>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct VariableKey {
    local: bool,
    scope: u64,
    context: u64,
    name: String,
}
#[derive(Clone, Debug)]
struct Identity {
    key: VariableKey,
    alias: Option<String>,
    list: bool,
    edit_range: Option<TextRange>,
}
#[derive(Clone, Debug)]
struct Fact {
    ty: InferredType,
    definitions: Vec<TextRange>,
    alias: Option<String>,
    list: bool,
}
type Facts = BTreeMap<VariableKey, Fact>;

#[derive(Clone, Debug)]
enum Reference {
    Variable {
        identity: Identity,
        reaching: Vec<TextRange>,
    },
    Function {
        definition: String,
        registration: String,
        witness: Option<FunctionWitness>,
    },
}
#[derive(Clone, Debug)]
struct FunctionWitness {
    name: String,
    signature: String,
    owner: String,
    revision: String,
    source: Option<String>,
}
#[derive(Clone, Debug)]
struct Occurrence {
    node: u64,
    range: TextRange,
    reference: Reference,
    ty: InferredType,
}
#[derive(Clone, Debug)]
struct Write {
    identity: Identity,
    range: TextRange,
}

// A bounded view at an authenticated variable evaluation boundary, after all
// preceding operands have already updated/invalidated the existing flow facts.
#[derive(Clone)]
pub(super) struct CompletionOwner {
    pub range: TextRange,
    pub identity: String,
    pub parameter: usize,
}
struct CompletionPoint {
    owner: CompletionOwner,
    range: TextRange,
    requested_types: Vec<String>,
    types: BTreeMap<String, InferredType>,
}
const MAX_COMPLETION_TYPE_BINDINGS: usize = 16_384;

pub(super) struct Document {
    pub uri: Uri,
    pub source_witness: String,
    pub project_complete: bool,
    rename_complete: bool,
    name_edit_certificate: Option<parser_wasm::host::StaticNameRenameCertificate>,
    function_policy: skript_parser::FunctionVersionPolicy,
    unknown_variable_scopes: HashSet<u64>,
    unavailable: Vec<TextRange>,
    native_owner: Option<String>,
    source: Arc<str>,
    lines: LineIndex,
    catalog: Arc<Catalog>,
    occurrences: Vec<Occurrence>,
    calls: Vec<call_hierarchy::CallSite>,
    symbols: Vec<lsp_types::DocumentSymbol>,
    // Lexical declaration information, never a replacement for current flow facts.
    declared_parameters: HashMap<VariableKey, InferredType>,
    completion_points: Vec<CompletionPoint>,
    completion_type_bindings: usize,
    writes: Vec<Write>,
    pub functions: Vec<FunctionSymbol>,
    pub dependencies: BTreeSet<Dependency>,
}

impl Document {
    pub fn from_result(
        result: &DocumentParseResult,
        source: Arc<str>,
        uri: Uri,
        catalog: Arc<Catalog>,
    ) -> Result<Self, String> {
        if result.source.original() != source.as_ref() {
            return Err("semantic source differs from parsed revision".into());
        }
        let lines = LineIndex::new(&source);
        let mut blocked = Vec::new();
        for span in result.unresolved_spans() {
            for origin in &span.origins {
                let range = TextRange::new(
                    usize::try_from(origin.original_range.start)
                        .map_err(|_| "semantic fault range overflow")?,
                    usize::try_from(origin.original_range.end)
                        .map_err(|_| "semantic fault range overflow")?,
                );
                lines.range(&source, range)?;
                blocked.push(range);
            }
        }
        let witnesses = result.semantic_witnesses();
        let mut functions = Vec::new();
        for registration in result.functions.registrations() {
            let declaration = &registration.declaration;
            if declaration.source != result.source.virtual_source() {
                continue;
            }
            // Registrations precede AST macros. Export only declarations whose
            // current structure still has the native semantic forest witness.
            let mut owners = result.ast.nodes.iter().filter(|node| {
                node.kind == SyntaxKind::Structure
                    && node.span.virtual_range.start <= declaration.span.start as u64
                    && node.span.virtual_range.end >= declaration.span.end as u64
                    && witnesses.native_valid(node)
            });
            let Some(owner) = owners.next() else {
                continue;
            };
            let body = owners
                .next()
                .is_none()
                .then(|| call_hierarchy::body(owner, &result.raw_tree, &lines, &source))
                .flatten();
            let Ok(span) = result.source.map_range(declaration.span) else {
                continue;
            };
            let [origin] = span.origins.as_slice() else {
                continue;
            };
            if origin.kind != skript_parser::OriginKind::Exact
                || blocked
                    .iter()
                    .any(|range| overlaps(*range, origin.original_range))
            {
                continue;
            }
            functions.push(FunctionSymbol {
                body,
                definition: registration.definition.clone(),
                range: lines.range(&source, origin.original_range)?,
                local: declaration.scope == FunctionScope::Local,
                name_range: (|| {
                    let start = declaration
                        .metadata
                        .get("function.name.start")?
                        .parse::<usize>()
                        .ok()?;
                    let end = declaration
                        .metadata
                        .get("function.name.end")?
                        .parse::<usize>()
                        .ok()?;
                    let (start, end) = match declaration
                        .metadata
                        .get("function.name.range-version")
                        .map(String::as_str)
                    {
                        None | Some("1") => (start, end),
                        Some("2") => (
                            declaration.span.start.checked_add(start)?,
                            declaration.span.start.checked_add(end)?,
                        ),
                        _ => return None,
                    };
                    if start < declaration.span.start
                        || end > declaration.span.end
                        || start >= end
                        || result.source.virtual_source().get(start..end)? != declaration.name
                    {
                        return None;
                    }
                    let span = result.source.map_range(TextRange::new(start, end)).ok()?;
                    let [origin] = span.origins.as_slice() else {
                        return None;
                    };
                    if origin.kind != skript_parser::OriginKind::Exact {
                        return None;
                    }
                    lines.range(&source, origin.original_range).ok()
                })(),
            });
        }
        let mut ast = result.ast.clone();
        let originals = result
            .ast
            .nodes
            .iter()
            .map(|node| (node.id, node))
            .collect::<HashMap<_, _>>();
        for node in &mut ast.nodes {
            let original = originals[&node.id];
            let provider = witnesses
                .unique_issuer(original)
                .filter(|issuer| witnesses.provider_valid(original, issuer));
            let trusted = witnesses.native_valid(original) || provider.is_some();
            let allowed = node
                .metadata
                .iter()
                .filter(|entry| {
                    matches!(
                        entry.key.as_str(),
                        CHANGE
                            | FLOW
                            | PARAMETERS
                            | LOCAL_EFFECTS
                            | OPERANDS
                            | VALUE_RESULT
                            | VARIABLE_READ
                            | EVALUATION_CONTEXT
                    )
                })
                .filter(|entry| {
                    entry
                        .owner_component_id
                        .as_deref()
                        .is_some_and(|issuer| witnesses.provider_valid(original, issuer))
                })
                .map(|entry| {
                    (
                        entry.key.clone(),
                        entry.value.clone(),
                        entry.owner_component_id.clone(),
                    )
                })
                .collect::<Vec<_>>();
            node.metadata.retain(|entry| {
                !matches!(
                    entry.key.as_str(),
                    CHANGE
                        | FLOW
                        | PARAMETERS
                        | LOCAL_EFFECTS
                        | OPERANDS
                        | VALUE_RESULT
                        | VARIABLE_READ
                        | EVALUATION_CONTEXT
                ) || allowed.contains(&(
                    entry.key.clone(),
                    entry.value.clone(),
                    entry.owner_component_id.clone(),
                ))
            });
            if !trusted {
                node.summary = None;
                node.function = None;
            } else if provider.is_none()
                && let Some(summary) = &mut node.summary
            {
                summary
                    .public_data
                    .retain(|record| record.schema_id != IDENTITY);
            }
        }
        let mut document = Self::from_ast(
            source,
            uri,
            catalog,
            &ast,
            &result.raw_tree,
            blocked,
            functions,
            Some(result.functions.document_id().to_owned()),
        )?;
        document.symbols = document_symbols::collect(result, &document.lines, |node, issuer| {
            witnesses.provider_valid(node, issuer)
        });
        for region in &result.dependency_unavailable {
            for origin in &region.span.origins {
                let range = TextRange::new(
                    origin
                        .original_range
                        .start
                        .try_into()
                        .map_err(|_| "dependency range overflow")?,
                    origin
                        .original_range
                        .end
                        .try_into()
                        .map_err(|_| "dependency range overflow")?,
                );
                document.lines.range(&document.source, range)?;
                document.unavailable.push(range);
            }
        }
        for lookup in result.functions.lookups() {
            document
                .dependencies
                .insert(Dependency::FunctionName(lookup.name.clone()));
            for definition in &lookup.definitions {
                document.dependencies.insert(Dependency::Function {
                    definition: definition.definition_id.clone(),
                    registration: definition.registration_id.clone(),
                });
            }
        }
        // Project discovery completeness alone does not certify reference coverage.
        // Recovery, faults and source-transforming macros require a richer rename
        // contract before they may participate in a source edit.
        document.rename_complete = result.function_namespace_complete
            && result.execution_status == parser_wasm::DocumentExecutionStatus::Complete
            && result.component_failures.is_empty()
            && result.unresolved_spans().next().is_none()
            && result.raw_tree.diagnostics.is_empty()
            && result.syntax.diagnostics.is_empty()
            && result.effects.diagnostics.iter().all(|diagnostic| {
                !matches!(
                    diagnostic.severity,
                    parser_wasm::host::DiagnosticSeverity::Error
                )
            })
            && result.source.virtual_source() == result.source.original()
            && result
                .text_macro_calls
                .iter()
                .all(|call| call.expansion.is_none())
            && result
                .tree_macro_calls
                .iter()
                .all(|call| call.expansion.is_none())
            && result
                .ast_macro_calls
                .iter()
                .all(|call| call.expansion.is_none());
        document.name_edit_certificate = result.static_name_rename.clone();
        document.function_policy = result.functions.policy();
        document.index_function_calls(&ast);
        Ok(document)
    }
    fn from_ast(
        source: Arc<str>,
        uri: Uri,
        catalog: Arc<Catalog>,
        ast: &AstTree,
        raw_tree: &skript_parser::RawTree,
        blocked: Vec<TextRange>,
        functions: Vec<FunctionSymbol>,
        native_owner: Option<String>,
    ) -> Result<Self, String> {
        let lines = LineIndex::new(&source);
        let mut document = Self {
            uri,
            source_witness: format!("{:x}", Sha256::digest(source.as_bytes())),
            native_owner,
            project_complete: true,
            rename_complete: false,
            name_edit_certificate: None,
            function_policy: skript_parser::FunctionVersionPolicy::default(),
            unknown_variable_scopes: HashSet::new(),
            unavailable: Vec::new(),
            source,
            lines,
            catalog,
            occurrences: Vec::new(),
            calls: Vec::new(),
            symbols: Vec::new(),
            declared_parameters: HashMap::new(),
            completion_points: Vec::new(),
            completion_type_bindings: 0,
            writes: Vec::new(),
            functions,
            dependencies: BTreeSet::new(),
        };
        let nodes = ast
            .nodes
            .iter()
            .map(|node| (node.id, node))
            .collect::<HashMap<_, _>>();
        if nodes.len() != ast.nodes.len() {
            return Err("duplicate semantic AST node ID".into());
        }
        let mut engine = Engine {
            document: &mut document,
            raw_tree,
            nodes,
            blocked,
            visiting: HashSet::new(),
            identities: HashMap::new(),
            pending_completion_owner: None,
            evaluation_context: None,
            context_preservation: HashMap::new(),
        };
        for root in &ast.roots {
            let mut facts = Facts::new();
            engine.node(*root, *root, &mut facts)?;
        }
        document
            .occurrences
            .sort_by_key(|occurrence| (occurrence.range.start, occurrence.range.end));
        document
            .completion_points
            .sort_by_key(|point| (point.range.start, point.range.end));
        Ok(document)
    }
    /// Only this exact revision's authenticated operand boundary can certify a
    /// candidate's stored local type. No previous occurrence is extrapolated.
    pub fn completion_input(
        &self,
        source: &str,
        edit: TextRange,
        name: &str,
    ) -> Option<(&InferredType, &[String], &CompletionOwner)> {
        if source != self.source.as_ref() || self.unavailable.iter().any(|r| overlaps(*r, edit)) {
            return None;
        }
        let index = self
            .completion_points
            .binary_search_by_key(&(edit.start, edit.end), |p| (p.range.start, p.range.end))
            .ok()?;
        if index
            .checked_sub(1)
            .and_then(|i| self.completion_points.get(i))
            .is_some_and(|p| p.range == edit)
            || self
                .completion_points
                .get(index + 1)
                .is_some_and(|p| p.range == edit)
        {
            return None;
        }
        let point = &self.completion_points[index];
        Some((point.types.get(name)?, &point.requested_types, &point.owner))
    }

    pub fn type_at(&self, position: Position) -> Option<(Range, &InferredType)> {
        let occurrence = self.at(position)?;
        if !matches!(occurrence.reference, Reference::Variable { .. }) {
            return None;
        }
        Some((
            self.lines.range(&self.source, occurrence.range).ok()?,
            &occurrence.ty,
        ))
    }
    pub fn hover(&self, position: Position) -> Option<(Range, String)> {
        let (range, ty) = self.type_at(position)?;
        let occurrence = self.at(position)?;
        let Reference::Variable { identity, .. } = &occurrence.reference else {
            return None;
        };
        let mut description = String::new();
        if let Some(declared) = self.declared_parameters.get(&identity.key) {
            description.push_str(&format!(
                "Declared parameter type: {}\n",
                declared.description(&self.catalog)
            ));
        }
        description.push_str(&format!("Inferred type: {}", ty.description(&self.catalog)));
        Some((range, description))
    }
    /// Project documents must all be current published generations. The caller
    /// removes invalidated documents before passing this read-only set.
    pub fn project_definition_unavailable(&self, position: Position) -> bool {
        !self.project_complete
            && (self
                .lines
                .offset(&self.source, position)
                .is_some_and(|offset| {
                    self.unavailable
                        .iter()
                        .any(|range| range.start <= offset && offset < range.end)
                })
                || self.at(position).is_some_and(|occurrence| {
                    matches!(
                        &occurrence.reference,
                        Reference::Function { .. }
                            | Reference::Variable {
                                identity: Identity {
                                    key: VariableKey { local: false, .. },
                                    ..
                                },
                                ..
                            }
                    )
                }))
    }
    pub fn definitions_at(&self, position: Position, project: &[&Document]) -> Vec<Location> {
        let Some(occurrence) = self.at(position) else {
            return Vec::new();
        };
        let mut locations = Vec::new();
        match &occurrence.reference {
            Reference::Function {
                definition,
                registration,
                witness,
            } => {
                for document in std::iter::once(self)
                    .chain(project.iter().copied().filter(|d| d.uri != self.uri))
                {
                    for symbol in &document.functions {
                        if symbol.definition.definition_id == *definition
                            && symbol.definition.registration_id == *registration
                            && witness.as_ref().is_some_and(|witness| {
                                witness.matches(&symbol.definition, &document.source_witness)
                            })
                            && (!symbol.local || document.uri == self.uri)
                        {
                            locations.push(Location::new(document.uri.clone(), symbol.range));
                        }
                    }
                }
            }
            Reference::Variable { identity, reaching } => {
                if identity.key.local && !reaching.is_empty() {
                    for range in reaching {
                        if let Ok(range) = self.lines.range(&self.source, *range) {
                            locations.push(Location::new(self.uri.clone(), range));
                        }
                    }
                } else {
                    for document in std::iter::once(self)
                        .chain(project.iter().copied().filter(|d| d.uri != self.uri))
                    {
                        if identity.key.local && document.uri != self.uri {
                            continue;
                        }
                        for write in &document.writes {
                            if definition_may_bind(&write.identity, identity) {
                                if let Ok(range) =
                                    document.lines.range(&document.source, write.range)
                                {
                                    locations.push(Location::new(document.uri.clone(), range));
                                }
                            }
                        }
                    }
                }
            }
        }
        locations.sort_by(|a, b| {
            (
                a.uri.as_str(),
                a.range.start.line,
                a.range.start.character,
                a.range.end.line,
                a.range.end.character,
            )
                .cmp(&(
                    b.uri.as_str(),
                    b.range.start.line,
                    b.range.start.character,
                    b.range.end.line,
                    b.range.end.character,
                ))
        });
        locations.dedup();
        locations
    }
    pub fn references_at(
        &self,
        position: Position,
        project: &[&Document],
        include_declaration: bool,
    ) -> Vec<Location> {
        let documents = std::iter::once(self)
            .chain(
                project
                    .iter()
                    .copied()
                    .filter(|document| document.uri != self.uri),
            )
            .collect::<Vec<_>>();
        let mut output = Vec::new();
        let at = self.at(position);
        let target = self.function_at(position, project);
        if let Some((owner, symbol)) = target {
            for document in &documents {
                for occurrence in &document.occurrences {
                    if let Reference::Function {
                        definition,
                        registration,
                        witness,
                    } = &occurrence.reference
                    {
                        if *definition == symbol.definition.definition_id
                            && *registration == symbol.definition.registration_id
                            && (!symbol.local || document.uri == owner.uri)
                            && witness.as_ref().is_some_and(|witness| {
                                witness.matches(&symbol.definition, &owner.source_witness)
                            })
                        {
                            if let Ok(range) =
                                document.lines.range(&document.source, occurrence.range)
                            {
                                output.push(Location::new(document.uri.clone(), range));
                            }
                        }
                    }
                }
            }
            if include_declaration {
                output.push(Location::new(
                    owner.uri.clone(),
                    symbol.name_range.unwrap_or(symbol.range),
                ));
            }
        } else if let Some(Occurrence {
            reference: Reference::Variable { identity, .. },
            ..
        }) = at
        {
            for document in &documents {
                if identity.key.local && document.uri != self.uri {
                    continue;
                }
                for occurrence in &document.occurrences {
                    let Reference::Variable {
                        identity: candidate,
                        ..
                    } = &occurrence.reference
                    else {
                        continue;
                    };
                    if candidate.key != identity.key {
                        continue;
                    }
                    let declaration = document.writes.iter().any(|write| {
                        write.range == occurrence.range && write.identity.key == identity.key
                    });
                    if declaration && !include_declaration {
                        continue;
                    }
                    if let Ok(range) = document.lines.range(&document.source, occurrence.range) {
                        output.push(Location::new(document.uri.clone(), range));
                    }
                }
            }
        }
        output.sort_by(|a, b| {
            (
                a.uri.as_str(),
                a.range.start.line,
                a.range.start.character,
                a.range.end.line,
                a.range.end.character,
            )
                .cmp(&(
                    b.uri.as_str(),
                    b.range.start.line,
                    b.range.start.character,
                    b.range.end.line,
                    b.range.end.character,
                ))
        });
        output.dedup();
        output
    }
    fn at(&self, position: Position) -> Option<&Occurrence> {
        let offset = self.lines.offset(&self.source, position)?;
        self.occurrences
            .iter()
            .filter(|occurrence| occurrence.range.start <= offset && offset < occurrence.range.end)
            .min_by_key(|occurrence| occurrence.range.end - occurrence.range.start)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Change {
    version: u32,
    mode: String,
    target_capture: u64,
    value_capture: Option<u64>,
    #[serde(default)]
    identity_delta: bool,
    #[serde(default)]
    identity_delta_types: Vec<String>,
    #[serde(default)]
    target_only: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
struct Operands {
    version: u32,
    #[serde(default)]
    prefix: Vec<u64>,
    #[serde(default)]
    required_context: Option<ContextRequirement>,
    #[serde(default)]
    steps: Vec<OperandStep>,
    /// Certifies all intrinsic postprocessing as well as every listed child;
    /// each callback between children must be represented by its step barrier.
    #[serde(default)]
    complete: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OperandStep {
    child_index: usize,
    /// Exact runtime classes whose intermediate processing preserves local facts.
    preserve_types: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextRequirement {
    key: String,
    value: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvaluationContext {
    version: u32,
    kind: String,
    key: Option<String>,
    value: Option<String>,
}
struct EvaluationStep {
    child: u64,
    preserve_types: Option<Vec<String>>,
}
struct EvaluationPlan {
    steps: Vec<EvaluationStep>,
    complete: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VariableRead {
    version: u32,
    requested_types: Vec<String>,
    may_refresh_player: bool,
    #[serde(default)]
    no_refresh_types: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Flow {
    version: u32,
    kind: String,
}

struct Engine<'a> {
    document: &'a mut Document,
    raw_tree: &'a skript_parser::RawTree,
    nodes: HashMap<u64, &'a AstNode>,
    blocked: Vec<TextRange>,
    visiting: HashSet<u64>,
    identities: HashMap<(u64, u64), Option<Identity>>,
    pending_completion_owner: Option<CompletionOwner>,
    evaluation_context: Option<(String, String, String)>,
    context_preservation: HashMap<u64, bool>,
}
impl Engine<'_> {
    fn node(&mut self, id: u64, scope: u64, facts: &mut Facts) -> Result<InferredType, String> {
        if !self.visiting.insert(id) {
            return Err("cyclic semantic AST".into());
        }
        let node = *self.nodes.get(&id).ok_or("missing semantic AST node")?;
        let completion_owner = self.pending_completion_owner.take();
        if node.kind == SyntaxKind::Structure {
            self.evaluation_context = unique_metadata(node, EVALUATION_CONTEXT)
                .and_then(|json| serde_json::from_str::<EvaluationContext>(json).ok())
                .filter(|claim| claim.version == 1 && claim.kind == "scope")
                .and_then(|claim| {
                    Some((
                        metadata_owner(node, EVALUATION_CONTEXT)?.to_owned(),
                        claim.key?,
                        claim.value?,
                    ))
                });
        } else if !self.preserves_evaluation_context(node, &mut HashSet::new()) {
            self.evaluation_context = None;
        }
        let answer = self.node_inner(node, scope, facts, completion_owner);
        self.visiting.remove(&id);
        answer
    }
    fn preserves_evaluation_context(
        &mut self,
        node: &AstNode,
        visiting: &mut HashSet<u64>,
    ) -> bool {
        if let Some(value) = self.context_preservation.get(&node.id) {
            return *value;
        }
        if !visiting.insert(node.id) {
            return false;
        }
        let intrinsic = unique_metadata(node, EVALUATION_CONTEXT)
            .and_then(|json| serde_json::from_str::<EvaluationContext>(json).ok())
            .is_some_and(|data| {
                data.version == 1
                    && data.kind == "preserve"
                    && data.key.is_none()
                    && data.value.is_none()
            });
        let grouped = node
            .summary
            .as_ref()
            .is_some_and(|summary| summary.kind == "grouped-expression")
            && node.children.len() == 1;
        let safe = self.exact(node).is_some()
            && (intrinsic || grouped)
            && node.children.iter().all(|id| {
                self.nodes
                    .get(id)
                    .copied()
                    .is_some_and(|child| self.preserves_evaluation_context(child, visiting))
            });
        visiting.remove(&node.id);
        self.context_preservation.insert(node.id, safe);
        safe
    }
    fn node_inner(
        &mut self,
        node: &AstNode,
        scope: u64,
        facts: &mut Facts,
        completion_owner: Option<CompletionOwner>,
    ) -> Result<InferredType, String> {
        let Some(range) = self.exact(node) else {
            facts.clear();
            return Ok(InferredType::Unknown);
        };
        if node
            .metadata
            .iter()
            .any(|entry| entry.key == "parser.editor-recovery")
        {
            facts.clear();
            return Ok(InferredType::Unknown);
        }
        if node.kind == SyntaxKind::Structure {
            self.parameters(node, scope, range, facts);
        }
        if node.kind == SyntaxKind::Expression
            && let Some(identity) = self.identity(node, scope)
        {
            let read = unique_metadata(node, VARIABLE_READ)
                .and_then(|json| serde_json::from_str::<VariableRead>(json).ok())
                .filter(|read| {
                    read.version == 1
                        && !read.requested_types.is_empty()
                        && node.children.is_empty()
                });
            if let Some(read) = &read
                && identity.key.local
                && !identity.list
                && let Some(owner) = completion_owner
            {
                let remaining = MAX_COMPLETION_TYPE_BINDINGS
                    .saturating_sub(self.document.completion_type_bindings);
                // Snapshot only a bounded set of types, never clone Facts or
                // declaration/alias histories at each statement. Budget overflow
                // yields Unknown for this boundary rather than a partial map.
                let mut types = BTreeMap::new();
                let mut complete = true;
                for (key, fact) in facts.iter().filter(|(key, _)| {
                    key.local
                        && key.scope == identity.key.scope
                        && key.context == identity.key.context
                }) {
                    if types.len() == remaining {
                        complete = false;
                        break;
                    }
                    types.insert(key.name.clone(), fact.ty.clone());
                }
                if complete && !types.is_empty() {
                    self.document.completion_type_bindings += types.len();
                    self.document.completion_points.push(CompletionPoint {
                        owner,
                        range,
                        requested_types: read.requested_types.clone(),
                        types,
                    });
                }
            }
            let fact = identity
                .key
                .local
                .then(|| facts.get(&identity.key))
                .flatten();
            // This is the stored value at the authenticated lookup boundary,
            // before a typed Variable invokes converters or Player refresh.
            let ty = if read.is_some() {
                fact.map_or(InferredType::Unknown, |fact| fact.ty.clone())
            } else {
                InferredType::Unknown
            };
            let reaching = if read.is_some() {
                fact.map_or_else(Vec::new, |fact| fact.definitions.clone())
            } else {
                Vec::new()
            };
            let safe = read
                .as_ref()
                .is_some_and(|read| read_preserves_type_facts(read, &ty, &self.document.catalog));
            if !identity.key.local {
                self.document
                    .dependencies
                    .insert(Dependency::GlobalVariable(identity.key.name.clone()));
                self.document
                    .dependencies
                    .insert(Dependency::UnknownVariableAlias);
            }
            self.document.occurrences.push(Occurrence {
                range,
                reference: Reference::Variable { identity, reaching },
                node: node.id,
                ty: ty.clone(),
            });
            if !safe {
                facts.clear();
            }
            for child in &node.children {
                self.node(*child, scope, &mut Facts::new())?;
            }
            return Ok(if safe { ty } else { InferredType::Unknown });
        }
        if node
            .summary
            .as_ref()
            .is_some_and(|summary| summary.kind == "variable")
        {
            self.document
                .dependencies
                .insert(Dependency::UnknownVariableAlias);
            self.document.unknown_variable_scopes.insert(scope);
        }
        if let Some(call) = &node.function {
            self.document.dependencies.insert(Dependency::Function {
                definition: call.definition_id.clone(),
                registration: call.registration_id.clone(),
            });
            if !call.name.is_empty() && node.text.starts_with(&call.name) {
                self.document.occurrences.push(Occurrence {
                    node: node.id,
                    range: TextRange::new(range.start, range.start + call.name.len()),
                    reference: Reference::Function {
                        definition: call.definition_id.clone(),
                        registration: call.registration_id.clone(),
                        witness: FunctionWitness::from_node(node, &call.name, self.document),
                    },
                    ty: known_type(node),
                });
            }
        }
        let body = body_nodes(node)?;
        let change = (node.kind == SyntaxKind::Effect)
            .then(|| {
                unique_metadata(node, CHANGE)
                    .and_then(|json| serde_json::from_str::<Change>(json).ok())
                    .filter(|change| matches!(change.version, 1 | 2))
            })
            .flatten();
        let nonbody = node
            .children
            .iter()
            .copied()
            .filter(|child| !body.contains(child))
            .collect::<Vec<_>>();
        // A native one-child group delegates directly. Structures are execution
        // domain containers; their body is handled separately below.
        let transparent = node.kind == SyntaxKind::Structure
            || (node
                .summary
                .as_ref()
                .is_some_and(|summary| summary.kind == "grouped-expression")
                && nonbody.len() == 1
                && body.is_empty());
        let prefix = if transparent {
            Some(EvaluationPlan {
                steps: nonbody
                    .iter()
                    .map(|child| EvaluationStep {
                        child: *child,
                        preserve_types: None,
                    })
                    .collect(),
                complete: true,
            })
        } else {
            operand_prefix(node, &body, &self.nodes, self.evaluation_context.as_ref())
        };
        let preserves = preserves_locals(node);
        if !transparent && prefix.is_none() && (!preserves || !nonbody.is_empty()) {
            facts.clear();
        }
        let mut child_types = HashMap::new();
        let plan = prefix.unwrap_or(EvaluationPlan {
            steps: Vec::new(),
            complete: false,
        });
        let ordered = plan.steps.iter().map(|step| step.child).collect::<Vec<_>>();
        for step in &plan.steps {
            self.pending_completion_owner = self.completion_owner(node, step.child);
            let ty = self.node(step.child, scope, facts)?;
            if let Some(classes) = &step.preserve_types
                && !matches!(&ty, InferredType::Values { classes: actual, maybe_unset: false, .. }
                    if !actual.is_empty() && actual.iter().all(|class| classes.contains(class)))
            {
                facts.clear();
            }
            child_types.insert(step.child, ty);
        }
        let unlisted = nonbody
            .iter()
            .filter(|child| !ordered.contains(child))
            .copied()
            .collect::<Vec<_>>();
        for child in &unlisted {
            // An opaque caller can evaluate these in any order, repeatedly, or
            // after a hidden callback. Never share operand-local state.
            child_types.insert(*child, self.node(*child, scope, &mut Facts::new())?);
        }
        let target_only = change.as_ref().is_some_and(|change| {
            let value = change
                .value_capture
                .and_then(|index| capture_node(node, index, &self.nodes))
                .and_then(|id| child_types.get(&id));
            change.version == 2
                && change.target_only
                && (change.mode == "delete"
                    || value.is_some_and(|value| identity_delta(change, value)))
        });
        if !transparent && !target_only && ((!preserves && !plan.complete) || !unlisted.is_empty())
        {
            facts.clear();
        }
        if node.kind == SyntaxKind::Effect {
            if let Some(change) = change {
                self.change(node, scope, facts, &change, &child_types)?;
            } else {
                // An unadvertised effect may invoke arbitrary addon code.
                facts.clear();
            }
        }
        if !body.is_empty() {
            if node.kind == SyntaxKind::Structure {
                self.body(node, &body, scope, facts)?;
            } else {
                let flow = unique_metadata(node, FLOW)
                    .and_then(|json| serde_json::from_str::<Flow>(json).ok())
                    .filter(|flow| flow.version == 1);
                if flow.as_ref().is_some_and(|flow| flow.kind == "branch") {
                    let before = facts.clone();
                    let mut branch = before.clone();
                    self.body(node, &body, scope, &mut branch)?;
                    *facts = join_facts(&before, &branch);
                } else {
                    // Loops may re-enter; other extension control flow is opaque.
                    let mut nested = Facts::new();
                    self.body(node, &body, scope, &mut nested)?;
                    facts.clear();
                }
            }
        }
        let ty = match node.summary.as_ref().map(|summary| summary.kind.as_str()) {
            Some("grouped-expression") if child_types.len() == 1 => {
                child_types.into_values().next().unwrap()
            }
            Some("expression-list") if !child_types.is_empty() => native_cardinality(
                child_types
                    .into_values()
                    .reduce(|left, right| left.join(&right))
                    .unwrap(),
                node,
            ),
            _ => known_type(node),
        };
        Ok(ty)
    }
    fn completion_owner(&self, parent: &AstNode, child: u64) -> Option<CompletionOwner> {
        if !matches!(parent.kind, SyntaxKind::Effect | SyntaxKind::Condition) {
            return None;
        }
        let summary = parent.summary.as_ref()?;
        let mut indices = parent.captures.iter().filter_map(|capture| {
            let (index, _) = capture.name.split_once(':')?;
            let index = index.parse::<u64>().ok()?;
            (capture_node(parent, index, &self.nodes) == Some(child)).then_some(index)
        });
        let index = indices.next()?;
        if indices.next().is_some() {
            return None;
        }
        Some(CompletionOwner {
            range: self.exact(parent)?,
            identity: format!(
                "{}:{}",
                summary.registration_id.as_ref()?,
                summary.pattern_index?
            ),
            parameter: usize::try_from(index).ok()?,
        })
    }
    fn body(
        &mut self,
        owner: &AstNode,
        ids: &[u64],
        scope: u64,
        facts: &mut Facts,
    ) -> Result<(), String> {
        let ordered = ids
            .iter()
            .map(|id| {
                let node = *self.nodes.get(id).ok_or("missing semantic body node")?;
                Ok((*id, self.exact(node).map(|range| range.start)))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let reordered = ordered
            .windows(2)
            .any(|pair| pair[0].1.is_none() || pair[1].1.is_none() || pair[0].1 >= pair[1].1);
        // Missing or rejected raw statements are barriers, even though no AST
        // node was published for them. Comments and blank lines are harmless.
        let raw_owner = self.raw_tree.nodes.iter().find(|raw| {
            raw.code_span.as_ref().is_some_and(|span| {
                usize::try_from(owner.span.virtual_range.start).ok()
                    == Some(span.virtual_range.start)
            })
        });
        let opaque = reordered || raw_owner.is_none() || raw_owner.into_iter().flat_map(|raw| raw.children.iter()).filter_map(|id| self.raw_tree.get(*id)).any(|raw| {
            !matches!(raw.kind, RawNodeKind::Blank | RawNodeKind::Comment) && raw.code_span.as_ref().is_none_or(|span| {
                !matches!(span.origins.as_slice(), [origin] if origin.kind == skript_parser::OriginKind::Exact)
            })
        });
        let mut barriers = raw_owner
            .into_iter()
            .flat_map(|raw| raw.children.iter())
            .filter_map(|id| self.raw_tree.get(*id))
            .filter(|raw| !matches!(raw.kind, RawNodeKind::Blank | RawNodeKind::Comment))
            .filter_map(|raw| {
                let range = raw.code_span.as_ref()?.origins.as_slice();
                let [origin] = range else {
                    return None;
                };
                (!ordered
                    .iter()
                    .any(|(_, start)| *start == Some(origin.original_range.start)))
                .then_some(origin.original_range.start)
            })
            .collect::<Vec<_>>();
        barriers.sort_unstable();
        let mut barrier = 0;
        for (id, start) in ordered {
            if opaque {
                facts.clear();
                self.evaluation_context = None;
            }
            while barriers
                .get(barrier)
                .is_some_and(|at| start.is_none_or(|start| *at < start))
            {
                facts.clear();
                self.evaluation_context = None;
                barrier += 1;
            }
            self.node(id, scope, facts)?;
        }
        if opaque || barrier < barriers.len() {
            facts.clear();
            self.evaluation_context = None;
        }
        Ok(())
    }
    fn change(
        &mut self,
        node: &AstNode,
        scope: u64,
        facts: &mut Facts,
        change: &Change,
        children: &HashMap<u64, InferredType>,
    ) -> Result<(), String> {
        if !matches!(
            change.mode.as_str(),
            "set" | "add" | "remove" | "remove-all" | "delete" | "reset"
        ) {
            facts.clear();
            return Ok(());
        }
        let Some(target_id) = capture_node(node, change.target_capture, &self.nodes) else {
            facts.clear();
            return Ok(());
        };
        let target = *self
            .nodes
            .get(&target_id)
            .ok_or("missing semantic change target")?;
        let Some(identity) = self.identity(target, scope) else {
            facts.clear();
            return Ok(());
        };
        let Some(range) = self.exact(target) else {
            facts.clear();
            return Ok(());
        };
        self.document.writes.push(Write {
            identity: identity.clone(),
            range,
        });
        let previous = facts.get(&identity.key).cloned();
        // A list/prefix or dynamic alias can affect any element family. Keep no
        // scalar facts across it until the provider publishes disjoint families.
        if identity.list {
            facts.clear();
        } else {
            facts.retain(|key, fact| {
                key == &identity.key
                    || (!fact.list && !may_alias(key, fact.alias.as_ref(), &identity))
            });
        }
        if !identity.key.local {
            return Ok(());
        }
        let value = change
            .value_capture
            .and_then(|index| capture_node(node, index, &self.nodes))
            .and_then(|id| children.get(&id))
            .cloned()
            .unwrap_or(InferredType::Unknown);
        let identity_delta = identity_delta(change, &value);
        let certified = change.version == 2
            && change.target_only
            && (change.mode == "delete" || identity_delta);
        let mut ty = match change.mode.as_str() {
            "set" if certified && identity_delta => value,
            "delete" if certified => InferredType::unset(),
            "add" if identity.list && certified && identity_delta => previous
                .as_ref()
                .map_or(InferredType::Unknown, |fact| fact.ty.join(&value)),
            "remove" | "remove-all" if identity.list && certified && identity_delta => previous
                .as_ref()
                .map_or(InferredType::Unknown, |fact| fact.ty.clone()),
            _ => InferredType::Unknown,
        };
        if identity.list
            && let InferredType::Values { multiple, .. } = &mut ty
        {
            *multiple = true;
        }
        if identity.list
            && matches!(change.mode.as_str(), "remove" | "remove-all")
            && let InferredType::Values { maybe_unset, .. } = &mut ty
        {
            *maybe_unset = true;
        }
        facts.insert(
            identity.key.clone(),
            Fact {
                ty: ty.clone(),
                definitions: vec![range],
                alias: identity.alias.clone(),
                list: identity.list,
            },
        );
        // The target occurrence describes the value after this accepted write.
        if let Some(occurrence) = self
            .document
            .occurrences
            .iter_mut()
            .rev()
            .find(|occurrence| occurrence.range == range)
        {
            occurrence.ty = ty;
            if let Reference::Variable { reaching, .. } = &mut occurrence.reference {
                *reaching = vec![range];
            }
        }
        Ok(())
    }
    fn identity(&mut self, node: &AstNode, scope: u64) -> Option<Identity> {
        let key = (node.id, scope);
        if let Some(value) = self.identities.get(&key) {
            return value.clone();
        }
        let answer = (|| {
            if node.summary.as_ref()?.kind != "variable" {
                return None;
            }
            let mut records = node
                .summary
                .as_ref()?
                .public_data
                .iter()
                .filter(|record| record.schema_id == IDENTITY);
            let record = records.next()?;
            if records.next().is_some() || record.schema_version != 1 {
                return None;
            }
            let data: IdentityData = serde_json::from_str(&record.json).ok()?;
            let edit_range = data.name_range.as_ref().and_then(|name| {
                if name.grammar != "ascii-identifier-v1" {
                    return None;
                }
                let base = self.exact(node)?;
                let start = base.start.checked_add(usize::try_from(name.start).ok()?)?;
                let end = base.start.checked_add(usize::try_from(name.end).ok()?)?;
                (base.start <= start
                    && start < end
                    && end <= base.end
                    && self.document.source.get(start..end) == Some(data.name.as_str()))
                .then_some(TextRange::new(start, end))
            });
            let mut identity = identity_data(data, scope, node.syntax_context)?;
            identity.edit_range = edit_range;
            Some(identity)
        })();
        self.identities.insert(key, answer.clone());
        answer
    }
    fn parameters(&mut self, node: &AstNode, scope: u64, header: TextRange, facts: &mut Facts) {
        let Some(data) = parameter_bindings(
            node.metadata
                .iter()
                .map(|entry| (entry.key.as_str(), entry.value.as_str())),
        ) else {
            return;
        };
        let Some(header_length) = node
            .span
            .virtual_range
            .end
            .checked_sub(node.span.virtual_range.start)
        else {
            return;
        };
        let mut accepted = Vec::new();
        let mut names = HashSet::new();
        for parameter in data.parameters {
            let Some(identity) = identity_data(parameter.identity, scope, node.syntax_context)
            else {
                return;
            };
            if !identity.key.local
                || !names.insert(identity.key.clone())
                || parameter.class_name.is_empty()
                || parameter.start >= parameter.end
                || (data.version == 1
                    && (parameter.start < node.span.virtual_range.start
                        || parameter.end > node.span.virtual_range.end))
                || (data.version == 2 && parameter.end > header_length)
            {
                return;
            }
            let relative_start = if data.version == 1 {
                parameter.start - node.span.virtual_range.start
            } else {
                parameter.start
            };
            let relative_end = if data.version == 1 {
                parameter.end - node.span.virtual_range.start
            } else {
                parameter.end
            };
            let Ok(start) = usize::try_from(relative_start) else {
                return;
            };
            let Ok(end) = usize::try_from(relative_end) else {
                return;
            };
            let Some(start) = header.start.checked_add(start) else {
                return;
            };
            let Some(end) = header.start.checked_add(end) else {
                return;
            };
            if end > header.end || self.document.source.get(start..end).is_none() {
                return;
            }
            let declared = InferredType::Values {
                classes: BTreeSet::from([parameter.class_name.clone()]),
                multiple: !parameter.single,
                maybe_unset: false,
            };
            let ty = if parameter.class_name == "java.lang.Object" {
                InferredType::Unknown
            } else {
                InferredType::Values {
                    classes: BTreeSet::from([parameter.class_name]),
                    multiple: !parameter.single,
                    maybe_unset: true,
                }
            };
            accepted.push((identity, TextRange::new(start, end), ty, declared));
        }
        for (identity, range, ty, declared) in accepted {
            self.document
                .declared_parameters
                .insert(identity.key.clone(), declared);
            facts.insert(
                identity.key.clone(),
                Fact {
                    ty: ty.clone(),
                    definitions: vec![range],
                    alias: identity.alias.clone(),
                    list: identity.list,
                },
            );
            self.document.writes.push(Write {
                identity: identity.clone(),
                range,
            });
            self.document.occurrences.push(Occurrence {
                range,
                node: node.id,
                reference: Reference::Variable {
                    identity,
                    reaching: vec![range],
                },
                ty,
            });
        }
    }
    fn exact(&self, node: &AstNode) -> Option<TextRange> {
        let range = exact_span(&node.span)?;
        (self.document.source.get(range.start..range.end)? == node.text
            && !self.blocked.iter().any(|blocked| overlaps(*blocked, range)))
        .then_some(range)
    }
}

impl FunctionWitness {
    fn from_node(node: &AstNode, name: &str, document: &Document) -> Option<Self> {
        let owner = unique_metadata(node, "function.document-id")?.to_owned();
        let source = if document.native_owner.as_deref() == Some(owner.as_str()) {
            Some(document.source_witness.clone())
        } else {
            unique_metadata(node, "function.source-witness").map(str::to_owned)
        };
        Some(Self {
            name: name.to_owned(),
            signature: unique_metadata(node, "function.signature")?.to_owned(),
            owner,
            revision: unique_metadata(node, "function.revision")?.to_owned(),
            source,
        })
    }
    fn matches(&self, definition: &FunctionDefinition, source: &str) -> bool {
        self.name == definition.name
            && self.signature == definition.signature_key()
            && definition.metadata.get("function.document-id") == Some(&self.owner)
            && (definition.metadata.get("function.revision") == Some(&self.revision)
                || self.source.as_deref() == Some(source))
    }
}

fn identity_data(data: IdentityData, scope: u64, context: u64) -> Option<Identity> {
    if data.resolution != "static"
        || data.name.is_empty()
        || !matches!(data.scope.as_str(), "local" | "global")
    {
        return None;
    }
    let local = data.scope == "local";
    Some(Identity {
        key: VariableKey {
            local,
            scope: if local { scope } else { 0 },
            context: if local { context } else { 0 },
            name: data.name,
        },
        alias: data.alias_key,
        list: data.list,
        edit_range: None,
    })
}

fn may_alias(key: &VariableKey, alias: Option<&String>, identity: &Identity) -> bool {
    if key.local != identity.key.local
        || key.scope != identity.key.scope
        || key.context != identity.key.context
    {
        return false;
    }
    match (alias, identity.alias.as_ref()) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    }
}
fn definition_may_bind(write: &Identity, read: &Identity) -> bool {
    if write.key.local != read.key.local
        || write.key.scope != read.key.scope
        || write.key.context != read.key.context
    {
        return false;
    }
    write.key.name == read.key.name
        || read.key.local
            && match (&write.alias, &read.alias) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            }
}
fn join_facts(left: &Facts, right: &Facts) -> Facts {
    left.iter()
        .filter_map(|(key, left)| {
            let right = right.get(key)?;
            let mut definitions = left.definitions.clone();
            definitions.extend(right.definitions.iter().copied());
            definitions.sort_by_key(|range| (range.start, range.end));
            definitions.dedup();
            Some((
                key.clone(),
                Fact {
                    ty: left.ty.join(&right.ty),
                    definitions,
                    alias: if left.alias == right.alias {
                        left.alias.clone()
                    } else {
                        None
                    },
                    list: left.list || right.list,
                },
            ))
        })
        .collect()
}
fn unique_metadata<'a>(node: &'a AstNode, key: &str) -> Option<&'a str> {
    let mut entries = node.metadata.iter().filter(|entry| entry.key == key);
    let value = entries.next()?;
    entries.next().is_none().then_some(value.value.as_str())
}
fn metadata_owner<'a>(node: &'a AstNode, key: &str) -> Option<&'a str> {
    let mut entries = node.metadata.iter().filter(|entry| entry.key == key);
    let entry = entries.next()?;
    if entries.next().is_some() {
        return None;
    }
    entry.owner_component_id.as_deref()
}
fn preserves_locals(node: &AstNode) -> bool {
    unique_metadata(node, LOCAL_EFFECTS)
        .and_then(|json| serde_json::from_str::<Flow>(json).ok())
        .is_some_and(|data| data.version == 1 && data.kind == "preserve")
}
fn identity_delta(change: &Change, value: &InferredType) -> bool {
    change.version == 2
        && (change.identity_delta
            || matches!(value,
        InferredType::Values { classes, .. } if !classes.is_empty() && classes.iter().all(|class| change.identity_delta_types.contains(class))))
}
fn operand_prefix(
    node: &AstNode,
    body: &[u64],
    nodes: &HashMap<u64, &AstNode>,
    context: Option<&(String, String, String)>,
) -> Option<EvaluationPlan> {
    let contract: Operands = serde_json::from_str(unique_metadata(node, OPERANDS)?).ok()?;
    if let Some(required) = &contract.required_context {
        let (owner, key, value) = context?;
        if metadata_owner(node, OPERANDS) != Some(owner.as_str())
            || &required.key != key
            || &required.value != value
        {
            return None;
        }
    }
    let mut result = Vec::new();
    match contract.version {
        1 if contract.steps.is_empty() && !contract.complete => {
            let mut indices = HashSet::new();
            let mut children = HashSet::new();
            for index in contract.prefix {
                if !indices.insert(index) {
                    return None;
                }
                let child = capture_node(node, index, nodes)?;
                if body.contains(&child)
                    || !children.insert(child)
                    || node.children.iter().filter(|id| **id == child).count() != 1
                {
                    return None;
                }
                result.push(EvaluationStep {
                    child,
                    preserve_types: None,
                });
            }
            Some(EvaluationPlan {
                steps: result,
                complete: false,
            })
        }
        2 if contract.prefix.is_empty() && body.is_empty() => {
            let parent = exact_span(&node.span)?;
            let mut previous_end = parent.start;
            // Intrinsic children have no pattern captures. Their current native
            // forest, exact projection and provider receipt bind these indices.
            for (index, step) in contract.steps.into_iter().enumerate() {
                if step.child_index != index {
                    return None;
                }
                let child = *node.children.get(index)?;
                if node.children.iter().filter(|id| **id == child).count() != 1 {
                    return None;
                }
                let child_node = nodes.get(&child)?;
                let range = exact_span(&child_node.span)?;
                if child_node.syntax_context != node.syntax_context
                    || range.start < previous_end
                    || range.end > parent.end
                    || range.start >= range.end
                {
                    return None;
                }
                previous_end = range.end;
                result.push(EvaluationStep {
                    child,
                    preserve_types: Some(step.preserve_types),
                });
            }
            if contract.complete && result.len() != node.children.len() {
                return None;
            }
            Some(EvaluationPlan {
                steps: result,
                complete: contract.complete,
            })
        }
        _ => None,
    }
}
fn read_preserves_type_facts(read: &VariableRead, ty: &InferredType, catalog: &Catalog) -> bool {
    let InferredType::Values {
        classes,
        maybe_unset: false,
        ..
    } = ty
    else {
        return false;
    };
    !classes.is_empty()
        && classes.iter().all(|class| {
            (!read.may_refresh_player || read.no_refresh_types.contains(class))
                && read
                    .requested_types
                    .iter()
                    .any(|requested| catalog.is_class_assignable(class, requested))
        })
}
pub(super) fn capture_node(
    node: &AstNode,
    index: u64,
    nodes: &HashMap<u64, &AstNode>,
) -> Option<u64> {
    if !source_geometry::pattern_capture_geometry_is_current(node) {
        return None;
    }
    let native = format!("{}{index}", source_geometry::PATTERN_CAPTURE_PREFIX);
    let mut slots = node
        .captures
        .iter()
        .filter(|capture| capture.name == native);
    let CaptureValue::Span(slot) = &slots.next()?.value else {
        return None;
    };
    if slots.next().is_some() {
        return None;
    }
    let prefix = format!("{index}:");
    let mut matches = node
        .captures
        .iter()
        .filter(|capture| capture.name.starts_with(&prefix));
    let capture = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    let CaptureValue::Node(id) = capture.value else {
        return None;
    };
    if !node.children.contains(&id) {
        return None;
    }
    let child = nodes.get(&id)?;
    (slot.virtual_range.start <= child.span.virtual_range.start
        && child.span.virtual_range.end <= slot.virtual_range.end)
        .then_some(id)
}
fn body_nodes(node: &AstNode) -> Result<Vec<u64>, String> {
    let mut captures = node
        .captures
        .iter()
        .filter(|capture| capture.name == "body");
    let Some(capture) = captures.next() else {
        return Ok(Vec::new());
    };
    if captures.next().is_some() {
        return Err("duplicate semantic body capture".into());
    }
    match &capture.value {
        CaptureValue::Nodes(ids) if ids.iter().all(|id| node.children.contains(id)) => {
            Ok(ids.clone())
        }
        _ => Err("semantic body capture references unavailable children".into()),
    }
}
fn known_type(node: &AstNode) -> InferredType {
    use parser_wasm::bindings::nlaocs::skript_parser_addon::types::DynamicMultiplicity;
    use parser_wasm::host::ExpressionPossibleReturnTypesState;
    let Some(summary) = &node.summary else {
        return InferredType::Unknown;
    };
    if summary.kind == "variable"
        || summary.possible_return_types_state != ExpressionPossibleReturnTypesState::Complete
    {
        return InferredType::Unknown;
    }
    let mut classes = summary
        .possible_return_types
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if classes.is_empty() {
        classes.extend(summary.return_type.iter().cloned());
    }
    if classes.is_empty() || classes.contains("java.lang.Object") {
        return InferredType::Unknown;
    }
    let Some(multiplicity) = summary.multiplicity else {
        return InferredType::Unknown;
    };
    if multiplicity == DynamicMultiplicity::Both {
        return InferredType::Unknown;
    }
    InferredType::Values {
        classes,
        multiple: multiplicity == DynamicMultiplicity::Multiple,
        maybe_unset: !unique_metadata(node, VALUE_RESULT)
            .and_then(|json| serde_json::from_str::<Flow>(json).ok())
            .is_some_and(|value| value.version == 1 && value.kind == "non-empty"),
    }
}
fn native_cardinality(mut ty: InferredType, node: &AstNode) -> InferredType {
    use parser_wasm::bindings::nlaocs::skript_parser_addon::types::DynamicMultiplicity;
    let multiplicity = node
        .summary
        .as_ref()
        .and_then(|summary| summary.multiplicity);
    let multiple = match multiplicity {
        Some(DynamicMultiplicity::Single) => false,
        Some(DynamicMultiplicity::Multiple) => true,
        _ => return InferredType::Unknown,
    };
    if let InferredType::Values {
        multiple: value, ..
    } = &mut ty
    {
        *value = multiple;
    }
    ty
}
fn exact_span(span: &MappedSpan) -> Option<TextRange> {
    use parser_wasm::bindings::nlaocs::skript_parser_addon::types::OriginKind;
    let [origin] = span.origins.as_slice() else {
        return None;
    };
    if origin.kind != OriginKind::Exact {
        return None;
    }
    Some(TextRange::new(
        usize::try_from(origin.original_range.start).ok()?,
        usize::try_from(origin.original_range.end).ok()?,
    ))
}
fn overlaps(a: TextRange, b: TextRange) -> bool {
    a.start < b.end && b.start < a.end
}

#[cfg(test)]
#[path = "semantic_tests.rs"]
mod tests;

mod diagnostics;

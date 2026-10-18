use super::positions::LineIndex;
use super::public_data::variable_details;
use lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind, Position};
use parser_wasm::{
    DocumentParseResult,
    bindings::nlaocs::skript_parser_addon::types::{DynamicMultiplicity, OriginKind},
    host::{AstNode, AstTree, MappedSpan},
};
use skript_parser::{FunctionDefinition, RawNodeKind, RawTriviaKind, TextRange};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};
use syntaxes::{Catalog, ParameterModifier, Syntax};

mod structure;

/// Only immutable data reaches the protocol thread. Native recursive candidates
/// and their speculative effects are destroyed on the parser worker's stack.
pub(super) struct Document {
    source: Arc<str>,
    lines: LineIndex,
    targets: Vec<Target>,
    blocked: Vec<TextRange>,
    format: MarkupKind,
}

#[derive(PartialEq, Eq)]
enum TargetId {
    Ast(u64),
    Entry(skript_parser::RawNodeId),
}
struct Target {
    id: TargetId,
    content: Arc<str>,
    depth: usize,
    origins: Vec<TextRange>,
}

impl Document {
    pub fn from_result(
        result: DocumentParseResult,
        source: Arc<str>,
        catalog: Arc<Catalog>,
        format: MarkupKind,
    ) -> Result<Self, String> {
        if source.as_ref() != result.source.original() {
            return Err("hover source does not match the parsed original revision".into());
        }
        let mut blocked = result
            .fault_regions
            .iter()
            .map(|region| origins(&region.span))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        for node in result.raw_tree.iter() {
            if matches!(node.kind, RawNodeKind::Comment | RawNodeKind::Invalid) {
                blocked.extend(
                    node.line
                        .content_span
                        .origins
                        .iter()
                        .map(|origin| origin.original_range),
                );
            }
            for trivia in &node.line.trailing_trivia {
                if matches!(
                    trivia.kind,
                    RawTriviaKind::LineComment | RawTriviaKind::BlockComment
                ) {
                    blocked.extend(
                        trivia
                            .span
                            .origins
                            .iter()
                            .map(|origin| origin.original_range),
                    );
                }
            }
        }
        let (entry_targets, entry_blocked) = structure::targets(&result, &format)?;
        blocked.extend(entry_blocked);
        let functions = result.functions.definitions();
        let mut document = Self::from_ast(source, result.ast, functions, catalog, blocked, format)?;
        for target in &entry_targets {
            for range in &target.origins {
                document.lines.range(&document.source, *range)?;
            }
        }
        document.targets.extend(entry_targets);
        Ok(document)
    }

    fn from_ast(
        source: Arc<str>,
        ast: AstTree,
        functions: Vec<FunctionDefinition>,
        catalog: Arc<Catalog>,
        mut blocked: Vec<TextRange>,
        format: MarkupKind,
    ) -> Result<Self, String> {
        let lines = LineIndex::new(&source);
        for range in &blocked {
            lines.range(&source, *range)?;
        }
        let mut nodes = BTreeMap::new();
        for node in ast.nodes {
            if nodes.insert(node.id, node).is_some() {
                return Err("duplicate hover AST node ID".into());
            }
        }
        let mut depths = HashMap::new();
        let mut pending = ast.roots.into_iter().map(|id| (id, 0)).collect::<Vec<_>>();
        while let Some((id, depth)) = pending.pop() {
            if depth > nodes.len() {
                return Err("cyclic hover AST".into());
            }
            if depths.get(&id).is_some_and(|previous| *previous >= depth) {
                continue;
            }
            let node = nodes.get(&id).ok_or("missing hover AST node")?;
            depths.insert(id, depth);
            pending.extend(node.children.iter().map(|child| (*child, depth + 1)));
        }
        let mut targets = Vec::new();
        let descriptions = Descriptions {
            functions,
            catalog,
            format: format.clone(),
        };
        let mut contents: HashSet<Arc<str>> = HashSet::new();
        for (id, depth) in depths {
            let node = nodes.remove(&id).unwrap();
            // Entry/group wrappers have no semantic summary of their own.
            if node.summary.is_none() {
                continue;
            }
            let all_origins = origins(&node.span)?;
            let mut exact = Vec::new();
            for (range, origin) in all_origins.into_iter().zip(&node.span.origins) {
                lines.range(&source, range)?;
                if origin.kind == OriginKind::Exact {
                    exact.push(range);
                } else {
                    // Expanded text can collapse several different semantic
                    // nodes onto one call-site span. Do not guess which one
                    // the original cursor denotes, or fall back to its parent.
                    blocked.push(range);
                }
            }
            if exact.is_empty() {
                continue;
            }
            let text = descriptions.describe(&node);
            let content = if let Some(shared) = contents.get(text.as_str()) {
                Arc::clone(shared)
            } else {
                let shared: Arc<str> = text.into();
                contents.insert(Arc::clone(&shared));
                shared
            };
            targets.push(Target {
                id: TargetId::Ast(node.id),
                content,
                depth,
                origins: exact,
            });
        }
        Ok(Self {
            source,
            lines,
            targets,
            blocked,
            format,
        })
    }

    pub fn at(&self, position: Position) -> Option<Hover> {
        let offset = self.lines.offset(&self.source, position)?;
        if self.source.get(offset..)?.chars().next()?.is_whitespace()
            || self
                .blocked
                .iter()
                .any(|range| contains(*range, offset) || range.is_empty() && range.start == offset)
        {
            return None;
        }
        let mut selected: Option<(&Target, TextRange)> = None;
        let mut ambiguous = false;
        for target in &self.targets {
            for range in &target.origins {
                if !contains(*range, offset) {
                    continue;
                }
                let rank = (range.end - range.start, std::cmp::Reverse(target.depth));
                match selected {
                    None => {
                        selected = Some((target, *range));
                        ambiguous = false;
                    }
                    Some((previous, previous_range)) => {
                        let previous_rank = (
                            previous_range.end - previous_range.start,
                            std::cmp::Reverse(previous.depth),
                        );
                        if rank < previous_rank {
                            selected = Some((target, *range));
                            ambiguous = false;
                        } else if rank == previous_rank && target.id != previous.id {
                            ambiguous = true;
                        }
                    }
                }
            }
        }
        if ambiguous {
            return None;
        }
        let (target, range) = selected?;
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: self.format.clone(),
                value: target.content.to_string(),
            }),
            range: Some(self.lines.range(&self.source, range).ok()?),
        })
    }
}

struct Descriptions {
    functions: Vec<FunctionDefinition>,
    catalog: Arc<Catalog>,
    format: MarkupKind,
}

impl Descriptions {
    fn describe(&self, node: &AstNode) -> String {
        let summary = node.summary.as_ref().unwrap();
        let variable = summary.kind == "variable";
        let syntax = match (&summary.registration_id, &summary.definition_id) {
            (Some(registration), Some(definition)) => {
                let mut matching = self
                    .catalog
                    .syntax_by_registration_id(registration)
                    .into_iter()
                    .filter(|syntax| {
                        syntax.definition_id().as_str() == definition
                            && same_kind(syntax.kind(), node.kind)
                    });
                let first = matching.next();
                if matching.next().is_none() {
                    first
                } else {
                    None
                }
            }
            _ => None,
        };
        let common = syntax.and_then(Syntax::common);
        let catalog_function = node.function.as_ref().and_then(|call| {
            let mut matching = self.catalog.functions().filter(|definition| {
                definition.name == call.name
                    && definition.definition_id.as_str() == call.definition_id
                    && definition.registration_id.as_str() == call.registration_id
            });
            let first = matching.next();
            if matching.next().is_none() {
                first
            } else {
                None
            }
        });
        let title = if let Some(function) = &node.function {
            format!("Function: {}", function.name)
        } else if variable {
            "Variable".into()
        } else {
            let name = common
                .and_then(|common| common.documentation.name.as_deref())
                .map(str::to_owned)
                .or_else(|| {
                    (node.kind == parser_wasm::host::SyntaxKind::Type)
                        .then(|| summary.return_type.as_deref().map(|ty| self.type_name(ty)))
                        .flatten()
                })
                .unwrap_or_else(|| node.syntax_id.clone());
            format!("{}: {name}", kind_name(node.kind))
        };
        let mut fields = Vec::new();
        if let Some(common) = common {
            fields.push((
                "Addon",
                format!("{} {}", common.addon.name, common.addon.version),
            ));
        } else if let Some(function) = catalog_function {
            fields.push((
                "Addon",
                format!("{} {}", function.addon.name, function.addon.version),
            ));
        }
        // A syntax implementation class is distinct from a Type's Java value
        // representation. Functions do not expose an implementation class here.
        let implementation = common
            .map(|common| common.element_class.as_str())
            .or_else(|| {
                (!matches!(
                    node.kind,
                    parser_wasm::host::SyntaxKind::Type | parser_wasm::host::SyntaxKind::Function
                ))
                .then_some(summary.element_class.as_deref())
                .flatten()
            });
        if let Some(class) = implementation {
            fields.push((
                "Java class",
                class.rsplit('.').next().unwrap_or(class).to_owned(),
            ));
            fields.push(("Qualified Java class", class.to_owned()));
        }
        if let Some(common) = common {
            if let Some(pattern) = summary
                .pattern_index
                .and_then(|index| usize::try_from(index).ok())
                .and_then(|index| common.patterns.get(index))
            {
                fields.push(("Pattern", pattern.source.clone()));
            }
            if common.deprecated == Some(true) {
                fields.push(("Deprecated", "yes".into()));
            }
        } else if let Some(pattern) = node
            .metadata
            .iter()
            .find(|entry| entry.key == "parser.pattern")
        {
            fields.push(("Pattern", pattern.value.clone()));
        }
        if variable {
            match variable_details(node) {
                Some((scope, template)) => {
                    fields.push(("Scope", scope));
                    fields.push(("Name template", template));
                }
                None => fields.push((
                    "Variable details",
                    "unavailable for this provider/schema".into(),
                )),
            }
        }
        if let Some(function) = &node.function {
            if let Some(definition) = self.functions.iter().find(|definition| {
                definition.name == function.name
                    && definition.definition_id == function.definition_id
                    && definition.registration_id == function.registration_id
            }) {
                let parameters = definition
                    .parameters
                    .iter()
                    .map(|parameter| {
                        self.parameter(
                            &parameter.name,
                            parameter.parameter_type.as_str(),
                            parameter.single,
                            parameter.optional,
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                fields.push((
                    "Signature",
                    format!(
                        "{}({parameters}) -> {}",
                        definition.name,
                        definition
                            .return_type
                            .as_ref()
                            .map_or_else(|| "none".into(), |ty| self.type_name(ty.as_str()))
                    ),
                ));
            } else if let Some(definition) = catalog_function {
                let parameters = definition
                    .parameters
                    .iter()
                    .map(|parameter| {
                        self.parameter(
                            &parameter.name,
                            parameter.parameter_type.as_str(),
                            parameter.single,
                            parameter.modifiers.contains(&ParameterModifier::Optional),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                fields.push((
                    "Signature",
                    format!(
                        "{}({parameters}) -> {}",
                        definition.name,
                        definition
                            .return_type
                            .as_ref()
                            .map_or_else(|| "none".into(), |ty| self.type_name(ty.as_str()))
                    ),
                ));
            }
        }
        if let Some(ty) = &summary.return_type {
            fields.push((
                if variable {
                    "Type (parse context, not inferred)"
                } else {
                    "Return type"
                },
                self.type_name(ty),
            ));
        }
        if summary.return_type.is_some() || variable {
            let multiplicity = match summary.multiplicity {
                Some(DynamicMultiplicity::Single) => "single",
                Some(DynamicMultiplicity::Multiple) => "multiple",
                Some(DynamicMultiplicity::Both) => "both",
                None => "unknown",
            };
            fields.push(("Multiplicity", multiplicity.into()));
        }
        if !summary.possible_return_types.is_empty()
            || !matches!(
                summary.possible_return_types_state,
                parser_wasm::host::ExpressionPossibleReturnTypesState::Complete
            )
        {
            let types = summary
                .possible_return_types
                .iter()
                .map(|ty| self.type_name(ty))
                .collect::<Vec<_>>()
                .join(", ");
            let label = match summary.possible_return_types_state {
                parser_wasm::host::ExpressionPossibleReturnTypesState::Complete => {
                    "Possible return types (complete)"
                }
                parser_wasm::host::ExpressionPossibleReturnTypesState::Partial => {
                    "Possible return types (partial)"
                }
                parser_wasm::host::ExpressionPossibleReturnTypesState::Unresolved => {
                    "Possible return types (unresolved)"
                }
            };
            fields.push((
                label,
                if types.is_empty() {
                    "unknown".into()
                } else {
                    types
                },
            ));
        }
        let documentation = catalog_function
            .map(|function| &function.documentation)
            .or_else(|| common.map(|common| &common.documentation))
            .or_else(|| {
                syntax.and_then(|syntax| match syntax {
                    Syntax::Type(ty) => Some(&ty.documentation),
                    Syntax::Function(function) => Some(&function.documentation),
                    _ => None,
                })
            });
        if let Some(documentation) = documentation {
            if !documentation.since.is_empty() {
                fields.push(("Since", documentation.since.join(", ")));
            }
            if !documentation.requires.is_empty() {
                fields.push(("Requires", documentation.requires.join(", ")));
            }
        }
        super::hover_text::render(&title, &fields, documentation, &self.format)
    }

    fn type_name(&self, class: &str) -> String {
        self.catalog
            .type_by_class_name(class)
            .map_or_else(|| class.to_owned(), |ty| ty.code_name.as_str().to_owned())
    }

    fn parameter(&self, name: &str, class: &str, single: bool, optional: bool) -> String {
        format!(
            "{name}: {}{}{}",
            self.type_name(class),
            if single { "" } else { " (multiple)" },
            if optional { " [optional]" } else { "" }
        )
    }
}

fn kind_name(kind: parser_wasm::host::SyntaxKind) -> &'static str {
    use parser_wasm::host::SyntaxKind;
    match kind {
        SyntaxKind::Event => "Event",
        SyntaxKind::Condition => "Condition",
        SyntaxKind::Effect => "Effect",
        SyntaxKind::Expression => "Expression",
        SyntaxKind::Type => "Type",
        SyntaxKind::Function => "Function",
        SyntaxKind::Section => "Section",
        SyntaxKind::Structure => "Structure",
    }
}

pub(super) fn same_kind(catalog: syntaxes::SyntaxKind, ast: parser_wasm::host::SyntaxKind) -> bool {
    use parser_wasm::host::SyntaxKind as Ast;
    use syntaxes::SyntaxKind as Catalog;
    matches!(
        (catalog, ast),
        (Catalog::Event, Ast::Event)
            | (Catalog::Condition, Ast::Condition)
            | (Catalog::Effect, Ast::Effect)
            | (Catalog::Expression, Ast::Expression)
            | (Catalog::Type, Ast::Type)
            | (Catalog::Function, Ast::Function)
            | (Catalog::Section, Ast::Section)
            | (Catalog::Structure, Ast::Structure)
    )
}

fn contains(range: TextRange, offset: usize) -> bool {
    range.start <= offset && offset < range.end
}

fn origins(span: &MappedSpan) -> Result<Vec<TextRange>, String> {
    if span.origins.is_empty() {
        return Err("hover node has no original source origin".into());
    }
    span.origins
        .iter()
        .map(|origin| {
            Ok(TextRange::new(
                usize::try_from(origin.original_range.start)
                    .map_err(|_| "hover source offset overflow")?,
                usize::try_from(origin.original_range.end)
                    .map_err(|_| "hover source offset overflow")?,
            ))
        })
        .collect()
}

#[cfg(test)]
impl Document {
    pub(super) fn test_fixture(source: Arc<str>) -> Self {
        let node = tests::node(71, &source, (0, source.len()));
        Self::from_ast(
            source,
            AstTree {
                roots: vec![71],
                nodes: vec![node],
            },
            Vec::new(),
            tests::catalog(),
            Vec::new(),
            MarkupKind::PlainText,
        )
        .expect("the controlled worker's synthetic hover AST must be valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::Range;
    use parser_wasm::bindings::nlaocs::skript_parser_addon::types::{
        AstContextOrigin, DynamicMultiplicity, ExpressionPossibleReturnTypesState,
        ExpressionPublicData, MetadataEntry, OriginKind, ParseSummary, SourceOrigin, SyntaxKind,
        TextRange as WitTextRange,
    };
    use syntax_pattern_parser::syntax::PluralRules;
    use syntaxes::{AliasRegistry, CatalogParts};

    pub(super) fn catalog() -> Arc<Catalog> {
        let plural_rules = PluralRules::from_json(
            r#"{
                "algorithm":"singular-aware",
                "pluralOverrideSupported":false,
                "rules":[{
                    "ruleOrder":0,
                    "singular":"",
                    "plural":"s",
                    "completeWord":false,
                    "origin":"built-in",
                    "addon":{"name":"Skript","version":"fixture"}
                }]
            }"#,
        )
        .unwrap();
        Arc::new(Catalog::new(CatalogParts {
            syntaxes: Vec::new(),
            converters: Vec::new(),
            comparators: Vec::new(),
            event_values: Vec::new(),
            properties: Vec::new(),
            operators: Vec::new(),
            operations: BTreeMap::new(),
            differences: Vec::new(),
            classes: Vec::new(),
            aliases: AliasRegistry {
                aliases: BTreeMap::new(),
                targets: Vec::new(),
            },
            plural_rules,
            language: BTreeMap::new(),
        }))
    }

    fn origin(start: usize, end: usize) -> SourceOrigin {
        SourceOrigin {
            original_range: WitTextRange {
                start: start as u64,
                end: end as u64,
            },
            kind: OriginKind::Exact,
            expansion: None,
        }
    }

    pub(super) fn node(id: u64, name: &str, range: (usize, usize)) -> AstNode {
        AstNode {
            id,
            kind: SyntaxKind::Expression,
            syntax_id: name.to_owned(),
            text: name.to_owned(),
            span: MappedSpan {
                virtual_range: WitTextRange {
                    start: range.0 as u64,
                    end: range.1 as u64,
                },
                origins: vec![origin(range.0, range.1)],
            },
            syntax_context: 0,
            context_origin: AstContextOrigin::Preserved,
            summary: Some(ParseSummary {
                kind: "literal".to_owned(),
                definition_id: None,
                registration_id: None,
                element_class: None,
                pattern_index: None,
                return_type: Some("java.lang.String".to_owned()),
                possible_return_types: Vec::new(),
                possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
                multiplicity: Some(DynamicMultiplicity::Single),
                default_expression: None,
                public_data: Vec::new(),
                metadata: Vec::new(),
            }),
            captures: Vec::new(),
            children: Vec::new(),
            function: None,
            metadata: Vec::new(),
        }
    }

    fn document(source: &str, roots: Vec<u64>, nodes: Vec<AstNode>) -> Document {
        Document::from_ast(
            Arc::from(source),
            AstTree { roots, nodes },
            Vec::new(),
            catalog(),
            Vec::new(),
            MarkupKind::PlainText,
        )
        .unwrap()
    }

    fn contents(hover: Hover) -> (String, Range) {
        let HoverContents::Markup(contents) = hover.contents else {
            panic!("hover must use MarkupContent");
        };
        assert_eq!(contents.kind, MarkupKind::PlainText);
        (
            contents.value,
            hover.range.expect("the original range is required"),
        )
    }

    fn assert_target(document: &Document, column: u32, name: &str, start: u32, end: u32) {
        let (text, range) = contents(document.at(Position::new(0, column)).unwrap());
        assert_eq!(
            text.lines().next(),
            Some(format!("Expression: {name}").as_str())
        );
        assert_eq!(
            range,
            Range::new(Position::new(0, start), Position::new(0, end))
        );
    }

    #[test]
    fn arbitrary_node_ids_select_the_shortest_origin_then_the_deepest_node() {
        let mut outer = node(9001, "outer", (0, 8));
        outer.children = vec![41];
        let mut inner = node(41, "inner", (2, 5));
        inner.children = vec![303];
        let deepest = node(303, "deepest", (2, 5));
        let shortest_root = node(777, "shortest", (3, 4));
        let document = document(
            "abcdefgh",
            vec![9001, 777],
            vec![deepest, shortest_root, outer, inner],
        );

        assert_target(&document, 2, "deepest", 2, 5);
        // A shorter root beats a deeper node covering a larger source range.
        assert_target(&document, 3, "shortest", 3, 4);
        // At the inner nodes' half-open end, only the ancestor still contains it.
        assert_target(&document, 5, "outer", 0, 8);
    }

    #[test]
    fn each_exact_original_origin_has_its_own_utf16_range_without_covering_the_gap() {
        let mut split = node(89, "split", (0, 16));
        split.span.origins = vec![origin(2, 8), origin(10, 14)];
        let document = document("ab日本--😀cd", vec![89], vec![split]);

        assert_target(&document, 2, "split", 2, 4);
        assert_target(&document, 6, "split", 6, 8);
        assert!(
            document.at(Position::new(0, 4)).is_none(),
            "the gap has no origin"
        );
        assert!(
            document.at(Position::new(0, 8)).is_none(),
            "origin ends are exclusive"
        );
    }

    #[test]
    fn equally_ranked_distinct_nodes_are_ambiguous_until_a_better_target_exists() {
        for roots in [vec![91, 17], vec![17, 91]] {
            let tied = document(
                "abc",
                roots.clone(),
                vec![node(17, "second", (0, 3)), node(91, "first", (0, 3))],
            );
            assert!(tied.at(Position::new(0, 1)).is_none());

            let mut with_shorter = roots;
            with_shorter.push(800);
            let resolved = document(
                "abc",
                with_shorter,
                vec![
                    node(17, "second", (0, 3)),
                    node(800, "unique-shortest", (1, 2)),
                    node(91, "first", (0, 3)),
                ],
            );
            assert_target(&resolved, 1, "unique-shortest", 1, 2);
        }
    }

    #[test]
    fn zero_width_exact_origins_are_not_targets_or_blocks_for_their_ancestor() {
        let anchor = node(27, "implicit", (1, 1));
        let only_anchor = document("abc", vec![27], vec![anchor.clone()]);
        assert!(only_anchor.at(Position::new(0, 1)).is_none());

        let mut outer = node(81, "outer", (0, 3));
        outer.children = vec![27];
        let nested = document("abc", vec![81], vec![anchor, outer]);
        assert_target(&nested, 1, "outer", 0, 3);
        assert!(nested.at(Position::new(0, 3)).is_none());
    }

    #[test]
    fn missing_origins_and_invalid_original_utf8_ranges_are_rejected() {
        for (origins, expected_error) in [
            (Vec::new(), "no original source origin"),
            (vec![origin(1, 3)], "invalid original UTF-8 offset"),
            (vec![origin(0, 2)], "invalid original UTF-8 offset"),
            (vec![origin(0, 5)], "invalid original UTF-8 offset"),
            (vec![origin(3, 0)], "reversed original range"),
        ] {
            let mut invalid = node(72, "invalid", (0, 4));
            invalid.span.origins = origins;
            let error = Document::from_ast(
                Arc::from("日x"),
                AstTree {
                    roots: vec![72],
                    nodes: vec![invalid],
                },
                Vec::new(),
                catalog(),
                Vec::new(),
                MarkupKind::PlainText,
            )
            .err()
            .expect("an unusable original source range must reject the hover document");
            assert!(error.contains(expected_error), "unexpected error: {error}");
        }
    }

    #[test]
    fn fault_regions_block_ancestors_only_within_the_fault_including_zero_width_faults() {
        let document = Document::from_ast(
            Arc::from("abcdef"),
            AstTree {
                roots: vec![45],
                nodes: vec![node(45, "outer", (0, 6))],
            },
            Vec::new(),
            catalog(),
            vec![TextRange::new(1, 3), TextRange::new(4, 4)],
            MarkupKind::PlainText,
        )
        .unwrap();

        for column in [1, 2, 4] {
            assert!(document.at(Position::new(0, column)).is_none());
        }
        for column in [0, 3, 5] {
            assert_target(&document, column, "outer", 0, 6);
        }
    }

    #[test]
    fn replaced_and_anchored_origins_do_not_fall_back_to_an_exact_ancestor() {
        for kind in [OriginKind::Replaced, OriginKind::Anchored] {
            let mut outer = node(51, "exact-ancestor", (0, 8));
            outer.children = vec![14];
            let mut mapped = node(14, "generated", (2, 4));
            mapped.span.origins[0].kind = kind;
            let document = document("abcdefgh", vec![51], vec![mapped, outer]);

            assert!(document.at(Position::new(0, 2)).is_none());
            assert!(document.at(Position::new(0, 3)).is_none());
            assert_target(&document, 1, "exact-ancestor", 0, 8);
            assert_target(&document, 4, "exact-ancestor", 0, 8);
        }
    }

    #[test]
    fn different_depths_collapsed_on_one_call_site_stay_unknown() {
        let mut outer = node(63, "exact-call-site", (0, 6));
        outer.children = vec![710];
        let mut generated = node(710, "generated-parent", (0, 6));
        generated.children = vec![19];
        generated.span.origins[0].kind = OriginKind::Replaced;
        generated.span.origins[0].expansion = Some(101);
        let mut deeper = node(19, "generated-child", (0, 6));
        deeper.span.origins[0].kind = OriginKind::Anchored;
        deeper.span.origins[0].expansion = Some(102);
        let unaffected = node(88, "unaffected", (7, 8));
        let document = document(
            "@macro x",
            vec![63, 88],
            vec![deeper, unaffected, generated, outer],
        );

        for column in [0, 1, 5] {
            assert!(
                document.at(Position::new(0, column)).is_none(),
                "depth cannot identify the cursor's generated node or justify ancestor fallback"
            );
        }
        assert_target(&document, 7, "unaffected", 7, 8);
    }

    fn variable(schema_id: &str, schema_version: u32) -> AstNode {
        let mut variable = node(71, "{_looks_local}", (0, 14));
        let summary = variable.summary.as_mut().unwrap();
        summary.kind = "variable".to_owned();
        summary.return_type = Some("java.lang.Object".to_owned());
        summary.public_data = vec![ExpressionPublicData {
            schema_id: schema_id.to_owned(),
            schema_version,
            json: r#"{"scope":"global","name":[{"kind":"text","text":"published-name"}]}"#
                .to_owned(),
        }];
        variable.metadata = vec![MetadataEntry {
            key: "variable.scope".to_owned(),
            value: "local".to_owned(),
            owner_component_id: None,
        }];
        variable
    }

    #[test]
    fn variable_v1_scope_and_name_come_from_public_data_rather_than_spelling() {
        let document = document(
            "{_looks_local}",
            vec![71],
            vec![variable("nlaocs.skript.variable", 1)],
        );
        let (text, range) = contents(document.at(Position::new(0, 3)).unwrap());

        assert_eq!(range, Range::new(Position::new(0, 0), Position::new(0, 14)));
        assert_eq!(text.lines().next(), Some("Variable"));
        assert!(text.lines().any(|line| line == "Scope: global"));
        assert!(
            text.lines()
                .any(|line| line == "Name template: published-name")
        );
        assert!(
            text.lines()
                .any(|line| { line == "Type (parse context, not inferred): java.lang.Object" })
        );
        assert!(!text.contains("Scope: local"));
        assert!(!text.contains("Return type:"));
    }

    #[test]
    fn unknown_variable_schema_or_version_does_not_guess_scope_or_name() {
        for (schema_id, schema_version) in
            [("nlaocs.skript.variable", 2), ("another-addon.variable", 1)]
        {
            let document = document(
                "{_looks_local}",
                vec![71],
                vec![variable(schema_id, schema_version)],
            );
            let (text, _) = contents(document.at(Position::new(0, 3)).unwrap());

            assert!(
                text.lines().any(|line| {
                    line == "Variable details: unavailable for this provider/schema"
                })
            );
            assert!(!text.contains("Scope:"), "scope was guessed: {text}");
            assert!(!text.contains("Name template:"), "name was guessed: {text}");
            assert!(!text.contains("published-name"));
            assert!(text.contains("Type (parse context, not inferred): java.lang.Object"));
        }
    }
}

//! Canonical post-parse AST exposed to AST macro components.
//!
//! The native parser keeps alternatives and recovery nodes in
//! [`StructureDocument`]. This module deliberately projects only selected
//! semantic nodes into the mutable WIT arena so a macro never has to understand
//! parser-internal candidate bookkeeping.

use std::collections::BTreeMap;

use skript_parser::{
    ConditionNode, ConditionNodeKind, EffectCandidate, EventCandidate, ExpressionNode,
    ExpressionNodeKind, MappedSource, MatchSpan, ParsedCapture, ParsedCaptureValue,
    SectionBodyNode, SectionCandidate, StructureBody, StructureCandidate, StructureDocument,
    StructureDocumentNode, StructureEntry, StructureEntryValue,
};
use syntaxes::{EntryKind, PossibleReturnTypesState};

use super::{
    AstContextOrigin, AstNode, AstTree, Capture, CaptureValue, SyntaxKind, WitMetadataEntry,
    WitParseSummary, WitPossibleReturnTypesState, expression_node_element_class,
    expression_node_identity, expression_node_registration, mapped_span_to_wit, metadata_to_wit,
    multiplicity_to_wit, public_data,
};

pub(super) fn from_structure_document(
    source: &MappedSource,
    document: &StructureDocument,
) -> AstTree {
    let mut builder = AstBuilder {
        source,
        nodes: Vec::new(),
    };
    let roots = document
        .roots
        .iter()
        .filter_map(|root| match root {
            StructureDocumentNode::Structure(matches) => matches
                .selected
                .as_ref()
                .map(|value| builder.structure(value)),
            StructureDocumentNode::Trivia(_) | StructureDocumentNode::Unclaimed(_) => None,
        })
        .collect();
    AstTree {
        roots,
        nodes: builder.nodes,
    }
}

struct AstBuilder<'a> {
    source: &'a MappedSource,
    nodes: Vec<AstNode>,
}

impl AstBuilder<'_> {
    fn structure(&mut self, structure: &StructureCandidate) -> u64 {
        let (mut captures, mut children) = self.parsed_captures(&structure.parsed_captures);
        match &structure.body {
            StructureBody::Entries(entries) => {
                for entry in entries {
                    children.push(self.entry(entry));
                }
            }
            StructureBody::Trigger(body) => children.extend(self.body(body)),
            StructureBody::None | StructureBody::Raw(_) => {}
        }
        captures.push(Capture {
            name: "body".to_owned(),
            value: CaptureValue::Nodes(children.clone()),
        });
        let metadata = candidate_metadata(
            &structure.matched,
            structure.metadata.clone(),
            [
                (
                    "parser.declared-node-type",
                    format!("{:?}", structure.declared_node_type),
                ),
                (
                    "parser.actual-node-type",
                    format!("{:?}", structure.actual_node_type),
                ),
            ],
        );
        self.push(NodeInput {
            kind: SyntaxKind::Structure,
            syntax_id: structure.matched.registration_id.clone(),
            span: &structure.matched.matched.span,
            summary: Some(candidate_summary(
                "structure",
                &structure.matched,
                structure.element_class.as_ref().map(|value| value.as_str()),
                metadata.clone(),
            )),
            captures,
            children,
            metadata,
        })
    }

    fn section(&mut self, section: &SectionCandidate) -> u64 {
        let (mut captures, mut children) = self.parsed_captures(&section.parsed_captures);
        let body = self.body(&section.body);
        children.extend(body.iter().copied());
        captures.push(Capture {
            name: "body".to_owned(),
            value: CaptureValue::Nodes(body),
        });
        let metadata = candidate_metadata(
            &section.matched,
            section.metadata.clone(),
            [
                ("parser.loop-section", section.loop_section.to_string()),
                ("parser.effect-section", section.effect_section.to_string()),
                (
                    "parser.section-expression",
                    section.section_expression.to_string(),
                ),
            ],
        );
        self.push(NodeInput {
            kind: SyntaxKind::Section,
            syntax_id: section.matched.registration_id.clone(),
            span: &section.matched.matched.span,
            summary: Some(candidate_summary(
                "section",
                &section.matched,
                section.element_class.as_ref().map(|value| value.as_str()),
                metadata.clone(),
            )),
            captures,
            children,
            metadata,
        })
    }

    fn effect(&mut self, effect: &EffectCandidate) -> u64 {
        let (captures, children) = self.parsed_captures(&effect.parsed_captures);
        let metadata = candidate_metadata(&effect.matched, effect.metadata.clone(), []);
        self.push(NodeInput {
            kind: SyntaxKind::Effect,
            syntax_id: effect.matched.registration_id.clone(),
            span: &effect.matched.matched.span,
            summary: Some(candidate_summary(
                "effect",
                &effect.matched,
                None,
                metadata.clone(),
            )),
            captures,
            children,
            metadata,
        })
    }

    fn event(&mut self, event: &EventCandidate) -> u64 {
        let metadata = candidate_metadata(
            &event.matched,
            event.metadata.clone(),
            [
                ("parser.cancellable", optional_bool(event.cancellable)),
                (
                    "parser.priority-supported",
                    optional_bool(event.priority_supported),
                ),
            ],
        );
        self.push(NodeInput {
            kind: SyntaxKind::Event,
            syntax_id: event.matched.registration_id.clone(),
            span: &event.span,
            summary: Some(candidate_summary(
                "event",
                &event.matched,
                event.element_class.as_ref().map(|value| value.as_str()),
                metadata.clone(),
            )),
            captures: Vec::new(),
            children: Vec::new(),
            metadata,
        })
    }

    fn condition(&mut self, condition: &ConditionNode) -> u64 {
        let mut captures = Vec::new();
        let mut children = Vec::new();
        for (index, expression) in condition.expressions.iter().enumerate() {
            let child = self.expression(expression);
            children.push(child);
            captures.push(Capture {
                name: format!("expression:{index}"),
                value: CaptureValue::Node(child),
            });
        }
        for (index, nested) in condition.children.iter().enumerate() {
            let child = self.condition(nested);
            children.push(child);
            captures.push(Capture {
                name: format!("condition:{index}"),
                value: CaptureValue::Node(child),
            });
        }

        let (syntax_id, summary, standard) = match &condition.kind {
            ConditionNodeKind::Grouped => {
                ("parser:condition.grouped".to_owned(), None, BTreeMap::new())
            }
            ConditionNodeKind::Registered {
                definition_id,
                registration_id,
                pattern_index,
                pattern,
                priority,
                registration_order,
            } => {
                let standard = BTreeMap::from([
                    ("parser.pattern".to_owned(), pattern.clone()),
                    ("parser.priority".to_owned(), priority.to_string()),
                    (
                        "parser.registration-order".to_owned(),
                        registration_order.to_string(),
                    ),
                ]);
                (
                    registration_id.clone(),
                    Some(WitParseSummary {
                        kind: "condition".to_owned(),
                        definition_id: Some(definition_id.clone()),
                        registration_id: Some(registration_id.clone()),
                        element_class: None,
                        pattern_index: u64::try_from(*pattern_index).ok(),
                        return_type: None,
                        possible_return_types: Vec::new(),
                        possible_return_types_state: WitPossibleReturnTypesState::Complete,
                        multiplicity: None,
                        public_data: Vec::new(),
                        metadata: metadata_to_wit(&condition.metadata),
                    }),
                    standard,
                )
            }
        };
        let mut metadata = condition.metadata.clone();
        metadata.extend(standard);
        self.push(NodeInput {
            kind: SyntaxKind::Condition,
            syntax_id,
            span: &condition.span,
            summary,
            captures,
            children,
            metadata: metadata_to_wit(&metadata),
        })
    }

    fn expression(&mut self, expression: &ExpressionNode) -> u64 {
        let (captures, children) = self.parsed_captures(&expression.parsed_captures());
        let (kind, syntax_id) = expression_syntax_identity(expression);
        let (definition_id, registration_id, pattern_index) =
            expression_node_registration(expression);
        let (summary_kind, _) = expression_node_identity(expression);
        let summary = WitParseSummary {
            kind: summary_kind.to_owned(),
            definition_id,
            registration_id,
            element_class: expression_node_element_class(expression, None),
            pattern_index,
            return_type: expression
                .return_type
                .as_ref()
                .map(|value| value.as_str().to_owned()),
            possible_return_types: expression
                .possible_return_types
                .iter()
                .map(|value| value.as_str().to_owned())
                .collect(),
            possible_return_types_state: possible_return_types_state(
                expression.possible_return_types_state,
            ),
            multiplicity: expression.multiplicity.map(multiplicity_to_wit),
            public_data: public_data::to_wit(&expression.public_data),
            metadata: metadata_to_wit(&expression.metadata),
        };
        self.push(NodeInput {
            kind,
            syntax_id,
            span: &expression.span,
            summary: Some(summary),
            captures,
            children,
            metadata: metadata_to_wit(&expression.metadata),
        })
    }

    fn entry(&mut self, entry: &StructureEntry) -> u64 {
        let mut captures = Vec::new();
        let mut children = Vec::new();
        match &entry.value {
            StructureEntryValue::Expression(expression) => {
                let child = self.expression(expression);
                children.push(child);
                captures.push(Capture {
                    name: "value".to_owned(),
                    value: CaptureValue::Node(child),
                });
            }
            StructureEntryValue::Trigger(body) => {
                let body = self.body(body);
                children.extend(body.iter().copied());
                captures.push(Capture {
                    name: "body".to_owned(),
                    value: CaptureValue::Nodes(body),
                });
            }
            StructureEntryValue::Container(entries) => {
                let entries = entries
                    .iter()
                    .map(|entry| self.entry(entry))
                    .collect::<Vec<_>>();
                children.extend(entries.iter().copied());
                captures.push(Capture {
                    name: "entries".to_owned(),
                    value: CaptureValue::Nodes(entries),
                });
            }
            StructureEntryValue::Raw(value) | StructureEntryValue::Unknown(value) => {
                captures.push(Capture {
                    name: "value".to_owned(),
                    value: CaptureValue::Text(value.clone()),
                });
            }
            StructureEntryValue::Section(_) => {}
        }
        let metadata = vec![
            metadata("parser.entry-key", entry.key.clone()),
            metadata("parser.entry-data-class", entry.entry_data_class.as_str()),
            metadata("parser.entry-kind", entry_kind(&entry.kind)),
            metadata("parser.entry-defaulted", entry.defaulted.to_string()),
        ];
        self.push(NodeInput {
            kind: SyntaxKind::Structure,
            syntax_id: format!("parser:structure-entry.{}", entry_kind(&entry.kind)),
            span: &entry.span,
            summary: None,
            captures,
            children,
            metadata,
        })
    }

    fn body(&mut self, body: &[SectionBodyNode]) -> Vec<u64> {
        body.iter()
            .filter_map(|node| match node {
                SectionBodyNode::Section(matches) => {
                    matches.selected.as_ref().map(|value| self.section(value))
                }
                SectionBodyNode::Effect(matches) => {
                    matches.selected.as_ref().map(|value| self.effect(value))
                }
                SectionBodyNode::Condition { matches, .. } => matches
                    .selected
                    .as_ref()
                    .map(|value| self.condition(&value.node)),
                SectionBodyNode::Trivia(_) | SectionBodyNode::Unclaimed(_) => None,
            })
            .collect()
    }

    fn parsed_captures(&mut self, values: &[ParsedCapture]) -> (Vec<Capture>, Vec<u64>) {
        let mut captures = Vec::with_capacity(values.len());
        let mut children = Vec::new();
        for capture in values {
            let node = match capture.result.value.as_ref() {
                Some(ParsedCaptureValue::Expression(value)) => Some(self.expression(value)),
                Some(ParsedCaptureValue::Condition(value)) => Some(self.condition(value)),
                Some(ParsedCaptureValue::Effect(value)) => Some(self.effect(value)),
                Some(ParsedCaptureValue::Event(value)) => Some(self.event(value)),
                Some(ParsedCaptureValue::Section(value)) => Some(self.section(value)),
                Some(ParsedCaptureValue::Raw(value)) => {
                    captures.push(Capture {
                        name: capture_name(capture),
                        value: CaptureValue::Text(value.clone()),
                    });
                    None
                }
                None => {
                    captures.push(Capture {
                        name: capture_name(capture),
                        value: CaptureValue::Span(mapped_span_to_wit(
                            capture.result.span.mapped.clone(),
                        )),
                    });
                    None
                }
            };
            if let Some(node) = node {
                children.push(node);
                captures.push(Capture {
                    name: capture_name(capture),
                    value: CaptureValue::Node(node),
                });
            }
        }
        (captures, children)
    }

    fn push(&mut self, input: NodeInput<'_>) -> u64 {
        let id = u64::try_from(self.nodes.len()).expect("AST node count exceeds u64");
        let syntax_context = input
            .span
            .mapped
            .primary_origin()
            .and_then(|origin| origin.expansion)
            .and_then(|expansion| self.source.expansions().get(expansion))
            .map_or(0, |expansion| u64::from(expansion.syntax_context.get()));
        self.nodes.push(AstNode {
            id,
            kind: input.kind,
            syntax_id: input.syntax_id,
            text: input
                .span
                .mapped
                .virtual_range
                .slice(self.source.virtual_source())
                .unwrap_or_default()
                .to_owned(),
            span: mapped_span_to_wit(input.span.mapped.clone()),
            syntax_context,
            context_origin: AstContextOrigin::Preserved,
            summary: input.summary,
            captures: input.captures,
            children: input.children,
            metadata: input.metadata,
        });
        id
    }
}

struct NodeInput<'a> {
    kind: SyntaxKind,
    syntax_id: String,
    span: &'a MatchSpan,
    summary: Option<WitParseSummary>,
    captures: Vec<Capture>,
    children: Vec<u64>,
    metadata: Vec<WitMetadataEntry>,
}

fn expression_syntax_identity(expression: &ExpressionNode) -> (SyntaxKind, String) {
    match &expression.kind {
        ExpressionNodeKind::Registered {
            registration_id, ..
        } => (SyntaxKind::Expression, registration_id.clone()),
        ExpressionNodeKind::Grouped => (
            SyntaxKind::Expression,
            "parser:expression.grouped".to_owned(),
        ),
        ExpressionNodeKind::List { .. } => {
            (SyntaxKind::Expression, "parser:expression.list".to_owned())
        }
        ExpressionNodeKind::Variable { parser_id } => (
            SyntaxKind::Expression,
            format!("parser:expression.{parser_id}"),
        ),
        ExpressionNodeKind::Literal { parser_id } => expression
            .metadata
            .get("nlaocs.core-library/type-parser-registration-id")
            .cloned()
            .map_or_else(
                || {
                    (
                        SyntaxKind::Expression,
                        format!("parser:expression.{parser_id}"),
                    )
                },
                |registration_id| (SyntaxKind::Type, registration_id),
            ),
        ExpressionNodeKind::Function { parser_id } => {
            (SyntaxKind::Function, format!("parser:function.{parser_id}"))
        }
        ExpressionNodeKind::Arithmetic {
            operation_registration_id,
            ..
        } => (
            SyntaxKind::Expression,
            format!("parser:arithmetic.{operation_registration_id}"),
        ),
        ExpressionNodeKind::Custom { parser_id } => (
            SyntaxKind::Expression,
            format!("parser:expression.{parser_id}"),
        ),
    }
}

fn candidate_summary(
    kind: &str,
    candidate: &skript_parser::CandidateMatch,
    element_class: Option<&str>,
    metadata: Vec<WitMetadataEntry>,
) -> WitParseSummary {
    WitParseSummary {
        kind: kind.to_owned(),
        definition_id: Some(candidate.definition_id.clone()),
        registration_id: Some(candidate.registration_id.clone()),
        element_class: element_class.map(str::to_owned),
        pattern_index: u64::try_from(candidate.pattern_index).ok(),
        return_type: None,
        possible_return_types: Vec::new(),
        possible_return_types_state: WitPossibleReturnTypesState::Complete,
        multiplicity: None,
        public_data: Vec::new(),
        metadata,
    }
}

fn candidate_metadata<const N: usize>(
    candidate: &skript_parser::CandidateMatch,
    mut metadata: BTreeMap<String, String>,
    extra: [(&str, String); N],
) -> Vec<WitMetadataEntry> {
    metadata.insert("parser.pattern".to_owned(), candidate.pattern.clone());
    metadata.insert("parser.priority".to_owned(), candidate.priority.to_string());
    metadata.insert(
        "parser.registration-order".to_owned(),
        candidate.registration_order.to_string(),
    );
    for (key, value) in extra {
        metadata.insert(key.to_owned(), value);
    }
    metadata_to_wit(&metadata)
}

fn possible_return_types_state(state: PossibleReturnTypesState) -> WitPossibleReturnTypesState {
    match state {
        PossibleReturnTypesState::Complete => WitPossibleReturnTypesState::Complete,
        PossibleReturnTypesState::Partial => WitPossibleReturnTypesState::Partial,
        PossibleReturnTypesState::Unresolved => WitPossibleReturnTypesState::Unresolved,
    }
}

fn metadata(key: impl Into<String>, value: impl Into<String>) -> WitMetadataEntry {
    WitMetadataEntry {
        owner_component_id: None,
        key: key.into(),
        value: value.into(),
    }
}

fn capture_name(capture: &ParsedCapture) -> String {
    format!("{}:{}", capture.capture_index, capture.binding.parser_id)
}

fn optional_bool(value: Option<bool>) -> String {
    value.map_or_else(|| "unresolved".to_owned(), |value| value.to_string())
}

fn entry_kind(kind: &EntryKind) -> &'static str {
    match kind {
        EntryKind::Literal => "literal",
        EntryKind::VariableString => "variable-string",
        EntryKind::Expression => "expression",
        EntryKind::Trigger => "trigger",
        EntryKind::Container => "container",
        EntryKind::Section => "section",
        EntryKind::KeyValue => "key-value",
        EntryKind::Unknown => "unknown",
    }
}

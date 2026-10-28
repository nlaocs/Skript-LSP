use super::*;
use parser_wasm::bindings::nlaocs::skript_parser_addon::types::{
    AstContextOrigin, Capture, DynamicMultiplicity, ExpressionPossibleReturnTypesState,
    ExpressionPublicData, MetadataEntry, OriginKind, ParseSummary, SourceOrigin,
    TextRange as WitRange,
};
use skript_parser::{MappedSource, PatternCapture, RawTreeOptions, TypeCaptureState};
use syntax_pattern_parser::syntax::{PatternTypeAlternative, PatternTypeExpr, Span};

use crate::lsp::test_support::multi_addon_catalog as catalog;
struct Fixture {
    source: Arc<str>,
    nodes: Vec<AstNode>,
}
impl Fixture {
    fn new(source: &str) -> Self {
        Self {
            source: Arc::from(source),
            nodes: Vec::new(),
        }
    }
    fn line(&self, row: usize) -> TextRange {
        let start = self
            .source
            .split_inclusive('\n')
            .take(row)
            .map(str::len)
            .sum::<usize>();
        let line = self.source[start..].lines().next().unwrap();
        TextRange::new(
            start + line.len() - line.trim_start().len(),
            start + line.len(),
        )
    }
    fn part(&self, row: usize, text: &str) -> TextRange {
        let line = self.line(row);
        let start = line.start + self.source[line.start..line.end].find(text).unwrap();
        TextRange::new(start, start + text.len())
    }
    fn node(
        &mut self,
        kind: SyntaxKind,
        semantic: &str,
        range: TextRange,
        class: Option<&str>,
    ) -> u64 {
        let id = self.nodes.len() as u64;
        let span = WitRange {
            start: range.start as u64,
            end: range.end as u64,
        };
        self.nodes.push(AstNode {
            id,
            kind,
            syntax_id: format!("fixture:{id}"),
            text: self.source[range.start..range.end].into(),
            span: MappedSpan {
                virtual_range: span.clone(),
                origins: vec![SourceOrigin {
                    original_range: span,
                    kind: OriginKind::Exact,
                    expansion: None,
                }],
            },
            syntax_context: 0,
            context_origin: AstContextOrigin::Preserved,
            summary: Some(ParseSummary {
                kind: semantic.into(),
                definition_id: None,
                registration_id: None,
                element_class: None,
                pattern_index: None,
                return_type: class.map(str::to_owned),
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
        });
        if semantic == "literal" {
            // The synthetic provider certifies this constructor, not arbitrary getters.
            for (key, value) in [
                (LOCAL_EFFECTS, json!({"version":1,"kind":"preserve"})),
                (VALUE_RESULT, json!({"version":1,"kind":"non-empty"})),
            ] {
                self.nodes[id as usize].metadata.push(MetadataEntry {
                    owner_component_id: Some("fixture.semantic-provider".into()),
                    key: key.into(),
                    value: value.to_string(),
                });
            }
        }
        id
    }
    fn variable(&mut self, row: usize) -> u64 {
        // A native variable's requested Location type is deliberately not its inferred value type.
        let id = self.node(
            SyntaxKind::Expression,
            "variable",
            self.part(row, "{_a}"),
            Some("org.bukkit.Location"),
        );
        self.nodes[id as usize].summary.as_mut().unwrap().public_data.push(ExpressionPublicData {
            schema_id: IDENTITY.into(), schema_version: 1,
            json: json!({"scope":"local","resolution":"static","name":"a","aliasKey":"a","list":false}).to_string(),
        });
        self.nodes[id as usize].metadata.push(MetadataEntry {
            owner_component_id: Some("fixture.semantic-provider".into()),
            key: VARIABLE_READ.into(),
            value:
                json!({"version":1,"requestedTypes":["org.bukkit.Location"],"mayRefreshPlayer":true,
                "noRefreshTypes":["java.lang.String","java.lang.Long"]})
                .to_string(),
        });
        id
    }
    fn seal(&mut self, owner: u64, slots: &[(u64, u64, &[&str])]) {
        let mapped = MappedSource::identity(Arc::clone(&self.source));
        let mut native = Vec::new();
        for (index, child, codes) in slots {
            let range = exact_span(&self.nodes[*child as usize].span).unwrap();
            self.nodes[owner as usize].children.push(*child);
            self.nodes[owner as usize].captures.push(Capture {
                name: format!("{index}:host.expression"),
                value: CaptureValue::Node(*child),
            });
            native.push(PatternCapture::TypeExpression {
                capture_index: *index as usize,
                state: TypeCaptureState::Explicit,
                omission: None,
                pattern_span: Span::new(0, 0),
                expression: PatternTypeExpr {
                    alternatives: codes
                        .iter()
                        .map(|code| PatternTypeAlternative {
                            name: (*code).into(),
                            plural: false,
                        })
                        .collect(),
                    nullable: false,
                    allow_literals: true,
                    allow_expressions: true,
                    time: 0,
                },
                value: self.source[range.start..range.end].into(),
                span: skript_parser::MatchSpan {
                    local_range: range,
                    mapped: mapped.map_range(range).unwrap(),
                },
                alternative_index: Some(0),
                resolution_id: Some(format!("fixture:{child}")),
            });
        }
        source_geometry::retain_pattern_captures(&mut self.nodes[owner as usize], &native);
    }
    fn assign(&mut self, row: usize, value: &str, class: &str) -> u64 {
        let variable = self.variable(row);
        // An lvalue is not a runtime value lookup. The write contract binds its identity.
        self.nodes[variable as usize]
            .metadata
            .retain(|entry| entry.key != VARIABLE_READ);
        let literal = self.node(
            SyntaxKind::Expression,
            "literal",
            self.part(row, value),
            Some(class),
        );
        let owner = self.node(SyntaxKind::Effect, "effect", self.line(row), None);
        self.nodes[owner as usize].metadata.push(MetadataEntry {
            owner_component_id: Some("fixture.semantic-provider".into()),
            key: CHANGE.into(),
            value: json!({"version":2,"mode":"set","targetCapture":0,"valueCapture":1,"identityDelta":true,"targetOnly":true}).to_string(),
        });
        self.seal(
            owner,
            &[(0, variable, &["object"]), (1, literal, &["object"])],
        );
        self.nodes[owner as usize].metadata.push(MetadataEntry {
            owner_component_id: Some("fixture.semantic-provider".into()),
            key: OPERANDS.into(),
            value: json!({"version":1,"prefix":[1]}).to_string(),
        });
        owner
    }
    fn teleport(&mut self, row: usize, codes: &[&str]) -> (u64, u64) {
        // This two-slot synthetic effect reads its destination first, then is opaque.
        // Actual Core Teleport has different capture indices and is tested end-to-end.
        let player = self.node(
            SyntaxKind::Expression,
            "literal",
            self.part(row, "player"),
            Some("org.bukkit.entity.Player"),
        );
        let variable = self.variable(row);
        let owner = self.node(SyntaxKind::Effect, "effect", self.line(row), None);
        self.seal(owner, &[(0, player, &["entity"]), (1, variable, codes)]);
        self.nodes[owner as usize].metadata.push(MetadataEntry {
            owner_component_id: Some("fixture.semantic-provider".into()),
            key: OPERANDS.into(),
            value: json!({"version":1,"prefix":[1]}).to_string(),
        });
        (owner, variable)
    }
    fn container(&mut self, row: usize, kind: SyntaxKind, children: &[u64], branch: bool) -> u64 {
        let id = self.node(kind, "owner", self.line(row), None);
        self.nodes[id as usize].children = children.to_vec();
        self.nodes[id as usize].captures.push(Capture {
            name: "body".into(),
            value: CaptureValue::Nodes(children.to_vec()),
        });
        if branch {
            // This synthetic branch has no header operands or intrinsic local writes.
            self.nodes[id as usize].metadata.push(MetadataEntry {
                owner_component_id: Some("fixture.semantic-provider".into()),
                key: LOCAL_EFFECTS.into(),
                value: json!({"version":1,"kind":"preserve"}).to_string(),
            });
            self.nodes[id as usize].metadata.push(MetadataEntry {
                owner_component_id: Some("fixture.semantic-provider".into()),
                key: FLOW.into(),
                value: json!({"version":1,"kind":"branch"}).to_string(),
            });
        }
        id
    }
    fn finish(self, root: u64) -> (Document, AstTree) {
        let ast = AstTree {
            roots: vec![root],
            nodes: self.nodes,
        };
        let raw = skript_parser::parse_raw_tree(
            &MappedSource::identity(Arc::clone(&self.source)),
            RawTreeOptions::for_skript_version(2, 15),
        );
        let document = Document::from_ast(
            self.source,
            "file:///inferred.sk".parse().unwrap(),
            catalog(),
            &ast,
            &raw,
            Vec::new(),
            Vec::new(),
            None,
            None,
            HashSet::new(),
            Vec::new(),
        )
        .unwrap();
        (document, ast)
    }
}

fn simple(class: &str, codes: &[&str]) -> (Document, AstTree, u64, u64) {
    let mut fixture = Fixture::new("owner:\n    set {_a} to value\n    teleport player to {_a}");
    let write = fixture.assign(1, "value", class);
    let (teleport, variable) = fixture.teleport(2, codes);
    let root = fixture.container(0, SyntaxKind::Structure, &[write, teleport], false);
    let (document, ast) = fixture.finish(root);
    (document, ast, teleport, variable)
}
fn diagnostics(document: &Document, ast: &AstTree) -> Vec<Diagnostic> {
    document
        .inferred_diagnostics(ast, &document.source, &[], |_| true, |_| true)
        .unwrap()
}

#[test]
fn inferred_variable_mismatch_reports_proven_type_without_rejecting_skript() {
    let (document, ast, _, variable) = simple("java.lang.String", &["location"]);
    let output = diagnostics(&document, &ast);
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].severity, Some(DiagnosticSeverity::WARNING));
    assert!(
        output[0]
            .message
            .contains("The parser accepted this reference")
    );
    assert!(!output[0].message.contains("at runtime"));
    assert_eq!(
        output[0].range,
        document
            .lines
            .range(
                &document.source,
                exact_span(&ast.nodes[variable as usize].span).unwrap()
            )
            .unwrap()
    );
    assert_eq!(
        output[0].data,
        Some(
            json!({"expectedTypes":["org.bukkit.Location"],"actualTypes":["java.lang.String"],"parserAccepted":true})
        )
    );
    assert_eq!(output[0].related_information.as_ref().unwrap().len(), 1);
}

#[test]
fn inferred_variable_conversions_alternative_types_and_unknown_expectations_suppress() {
    for (class, codes) in [
        ("org.bukkit.entity.Player", vec!["location"]),
        ("java.lang.String", vec!["location", "string"]),
        ("java.lang.String", vec!["location", "unavailabletype"]),
    ] {
        let (document, ast, _, _) = simple(class, &codes);
        assert!(diagnostics(&document, &ast).is_empty(), "{class} {codes:?}");
    }
}

#[test]
fn inferred_variable_reassignment_and_branch_unions_use_current_reaching_types() {
    for branch in [false, true] {
        let mut fixture = Fixture::new(
            "owner:\n    set {_a} to text\n    branch:\n        set {_a} to actor\n    teleport player to {_a}",
        );
        let first = fixture.assign(1, "text", "java.lang.String");
        let second = fixture.assign(3, "actor", "org.bukkit.entity.Player");
        let nested = fixture.container(
            2,
            if branch {
                SyntaxKind::Section
            } else {
                SyntaxKind::Structure
            },
            &[second],
            branch,
        );
        let (teleport, _) = fixture.teleport(4, &["location"]);
        let root = fixture.container(0, SyntaxKind::Structure, &[first, nested, teleport], false);
        let (document, ast) = fixture.finish(root);
        assert!(
            diagnostics(&document, &ast).is_empty(),
            "a reachable Player conversion is valid"
        );
    }
}

#[test]
fn inferred_variable_wholly_incompatible_branch_union_reports_every_reaching_type() {
    let mut fixture = Fixture::new(
        "owner:\n    set {_a} to text\n    branch:\n        set {_a} to count\n    teleport player to {_a}",
    );
    let first = fixture.assign(1, "text", "java.lang.String");
    let second = fixture.assign(3, "count", "java.lang.Number");
    let branch = fixture.container(2, SyntaxKind::Section, &[second], true);
    let (teleport, _) = fixture.teleport(4, &["location"]);
    let root = fixture.container(0, SyntaxKind::Structure, &[first, branch, teleport], false);
    let (document, ast) = fixture.finish(root);
    let output = diagnostics(&document, &ast);
    assert_eq!(output.len(), 1);
    assert_eq!(
        output[0].data,
        Some(json!({"expectedTypes":["org.bukkit.Location"],
        "actualTypes":["java.lang.Number","java.lang.String"],"parserAccepted":true}))
    );
    assert_eq!(output[0].related_information.as_ref().unwrap().len(), 2);
}

#[test]
fn inferred_variable_requires_native_slot_and_identity_provider_witnesses() {
    let (document, ast, _, _) = simple("java.lang.String", &["location"]);
    assert!(
        document
            .inferred_diagnostics(&ast, &document.source, &[], |_| false, |_| true)
            .unwrap()
            .is_empty()
    );
    assert!(
        document
            .inferred_diagnostics(&ast, &document.source, &[], |_| true, |_| false)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn inferred_variable_opaque_or_unordered_parent_clears_facts_before_operand_lookup() {
    for mode in ["no-prefix", "unknown-first", "repeated"] {
        let mut fixture =
            Fixture::new("owner:\n    set {_a} to value\n    teleport player to {_a}");
        let write = fixture.assign(1, "value", "java.lang.String");
        let (teleport, _) = fixture.teleport(2, &["location"]);
        let actor = fixture.nodes[teleport as usize].children[0];
        match mode {
            "no-prefix" => fixture.nodes[teleport as usize]
                .metadata
                .retain(|entry| entry.key != OPERANDS),
            "unknown-first" => {
                fixture.nodes[teleport as usize]
                    .metadata
                    .iter_mut()
                    .find(|entry| entry.key == OPERANDS)
                    .unwrap()
                    .value = json!({"version":1,"prefix":[0,1]}).to_string();
                fixture.nodes[actor as usize].summary.as_mut().unwrap().kind =
                    "registered-expression".into();
                fixture.nodes[actor as usize]
                    .metadata
                    .retain(|entry| entry.key != LOCAL_EFFECTS && entry.key != VALUE_RESULT);
            }
            "repeated" => {
                fixture.nodes[teleport as usize]
                    .metadata
                    .iter_mut()
                    .find(|entry| entry.key == OPERANDS)
                    .unwrap()
                    .value = json!({"version":1,"prefix":[1,1]}).to_string()
            }
            _ => unreachable!(),
        }
        let root = fixture.container(0, SyntaxKind::Structure, &[write, teleport], false);
        let (document, ast) = fixture.finish(root);
        assert!(diagnostics(&document, &ast).is_empty(), "{mode}");
    }
}

#[test]
fn inferred_variable_unknown_call_or_rejected_raw_statement_is_a_barrier() {
    for keep in [false, true] {
        let mut fixture =
            Fixture::new("owner:\n    set {_a} to value\n    opaque\n    teleport player to {_a}");
        let write = fixture.assign(1, "value", "java.lang.String");
        let mut body = vec![write];
        if keep {
            body.push(fixture.node(SyntaxKind::Function, "function", fixture.line(2), None));
        }
        let (teleport, _) = fixture.teleport(3, &["location"]);
        body.push(teleport);
        let root = fixture.container(0, SyntaxKind::Structure, &body, false);
        let (document, ast) = fixture.finish(root);
        assert!(diagnostics(&document, &ast).is_empty());
    }
}

#[test]
fn inferred_variable_edited_seal_child_context_and_faults_do_not_claim_types() {
    for mutation in ["expected", "context", "fault", "dynamic"] {
        let (document, mut ast, owner, variable) = simple("java.lang.String", &["location"]);
        let mut blocked = Vec::new();
        match mutation {
            "expected" => {
                ast.nodes[owner as usize]
                    .captures
                    .iter_mut()
                    .find(|capture| {
                        capture
                            .name
                            .starts_with(source_geometry::PATTERN_EXPECTED_TYPES_PREFIX)
                    })
                    .unwrap()
                    .value = CaptureValue::Text("[\"string\"]".into())
            }
            "context" => ast.nodes[variable as usize].syntax_context = 99,
            "fault" => blocked.push(exact_span(&ast.nodes[variable as usize].span).unwrap()),
            "dynamic" => {
                ast.nodes[variable as usize]
                    .summary
                    .as_mut()
                    .unwrap()
                    .public_data[0]
                    .json = json!({"scope":"local","resolution":"dynamic","name":"a","list":false})
                    .to_string()
            }
            _ => unreachable!(),
        }
        assert!(
            document
                .inferred_diagnostics(&ast, &document.source, &blocked, |_| true, |_| true)
                .unwrap()
                .is_empty(),
            "{mutation}"
        );
    }
}

#[test]
fn inferred_variable_unknown_or_unset_facts_and_changed_sources_are_not_reused() {
    for ty in [
        InferredType::Unknown,
        InferredType::unset(),
        InferredType::Values {
            classes: BTreeSet::from(["java.lang.Object".into()]),
            multiple: false,
            maybe_unset: false,
            support: TypeSupport::inferred(),
        },
        InferredType::Values {
            classes: BTreeSet::from(["java.lang.String".into()]),
            multiple: false,
            maybe_unset: true,
            support: TypeSupport::inferred(),
        },
    ] {
        let (mut document, ast, _, variable) = simple("java.lang.String", &["location"]);
        let range = exact_span(&ast.nodes[variable as usize].span).unwrap();
        for occurrence in &mut document.occurrences {
            if occurrence.range == range {
                occurrence.ty = ty.clone();
            }
        }
        assert!(diagnostics(&document, &ast).is_empty());
    }
    let (document, ast, _, _) = simple("java.lang.String", &["location"]);
    assert!(
        document
            .inferred_diagnostics(&ast, "different revision", &[], |_| true, |_| true)
            .is_err()
    );
}

#[test]
fn heuristic_and_partial_type_candidates_never_issue_definite_mismatch_warnings() {
    for certainty in [
        Certainty::Inferred,
        Certainty::Heuristic,
        Certainty::Unknown,
    ] {
        for complete in [false, true] {
            let (mut document, ast, _, variable) = simple("java.lang.String", &["location"]);
            let range = exact_span(&ast.nodes[variable as usize].span).unwrap();
            for occurrence in &mut document.occurrences {
                if occurrence.range == range {
                    let InferredType::Values { support, .. } = &mut occurrence.ty else {
                        panic!()
                    };
                    support.certainty = certainty;
                    support.candidates_complete = complete;
                }
            }
            assert_eq!(
                !diagnostics(&document, &ast).is_empty(),
                certainty == Certainty::Inferred && complete,
                "{certainty:?} complete={complete}"
            );
        }
    }
}

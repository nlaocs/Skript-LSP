use super::*;
use parser_wasm::bindings::nlaocs::skript_parser_addon::types::{
    AstContextOrigin, Capture, DynamicMultiplicity, ExpressionPossibleReturnTypesState,
    ExpressionPublicData, FunctionCall, MetadataEntry, OriginKind, ParseSummary, SourceOrigin,
    TextRange as WitRange,
};
use serde_json::json;
use std::sync::LazyLock;

fn catalog() -> Arc<Catalog> {
    static CATALOG: LazyLock<Arc<Catalog>> = LazyLock::new(|| {
        Arc::new(
            ssg::load(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4"),
            )
            .unwrap()
            .into_catalog(),
        )
    });
    Arc::clone(&CATALOG)
}
fn metadata(key: &str, value: serde_json::Value) -> MetadataEntry {
    MetadataEntry {
        owner_component_id: Some("fixture.semantic-provider".into()),
        key: key.into(),
        value: value.to_string(),
    }
}
fn node(source: &str, id: u64, kind: SyntaxKind, semantic: &str, range: TextRange) -> AstNode {
    let mapped = WitRange {
        start: range.start as u64,
        end: range.end as u64,
    };
    AstNode {
        id,
        kind,
        syntax_id: format!("fixture:{id}"),
        text: source[range.start..range.end].into(),
        span: MappedSpan {
            virtual_range: mapped.clone(),
            origins: vec![SourceOrigin {
                original_range: mapped,
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
            return_type: None,
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
    fn push(&mut self, kind: SyntaxKind, semantic: &str, range: TextRange) -> u64 {
        let id = self.nodes.len() as u64;
        self.nodes
            .push(node(&self.source, id, kind, semantic, range));
        id
    }
    fn var(&mut self, row: usize, spelling: &str, name: &str, local: bool, list: bool) -> u64 {
        let id = self.push(SyntaxKind::Expression, "variable", self.part(row, spelling));
        self.nodes[id as usize].summary.as_mut().unwrap().public_data.push(ExpressionPublicData {
            schema_id: IDENTITY.into(), schema_version: 1,
            json: json!({"scope":if local {"local"} else {"global"},"resolution":"static","name":name,"aliasKey":name.is_ascii().then(||name.to_ascii_lowercase()),"list":list,"casePolicy":"unknown"}).to_string(),
        });
        self.nodes[id as usize].metadata.push(metadata(
            VARIABLE_READ,
            json!({
                "version":1,"requestedTypes":["java.lang.Object"],"mayRefreshPlayer":true,
                "noRefreshTypes":["java.lang.Number","java.lang.String"]
            }),
        ));
        id
    }
    fn seal(&mut self, id: u64, slots: &[(u64, u64)]) {
        let mapped = skript_parser::MappedSource::identity(Arc::clone(&self.source));
        let mut native = Vec::new();
        for (index, child) in slots {
            let span = exact_span(&self.nodes[*child as usize].span).unwrap();
            self.nodes[id as usize].children.push(*child);
            self.nodes[id as usize].captures.push(Capture {
                name: format!("{index}:fixture.expression"),
                value: CaptureValue::Node(*child),
            });
            native.push(skript_parser::PatternCapture::Regex {
                capture_index: *index as usize,
                pattern_span: syntax_pattern_parser::syntax::Span::new(0, 0),
                value: self.source[span.start..span.end].into(),
                span: skript_parser::MatchSpan {
                    local_range: span,
                    mapped: mapped.map_range(span).unwrap(),
                },
                groups: Vec::new(),
            });
        }
        source_geometry::retain_pattern_captures(&mut self.nodes[id as usize], &native);
    }
    fn assign(
        &mut self,
        row: usize,
        spelling: &str,
        name: &str,
        value: &str,
        class: &str,
        local: bool,
        list: bool,
        mode: &str,
    ) -> u64 {
        let variable = self.var(row, spelling, name, local, list);
        let literal = self.push(SyntaxKind::Expression, "literal", self.part(row, value));
        self.nodes[literal as usize]
            .summary
            .as_mut()
            .unwrap()
            .return_type = Some(class.into());
        self.nodes[literal as usize].metadata.push(metadata(
            LOCAL_EFFECTS,
            json!({"version":1,"kind":"preserve"}),
        ));
        self.nodes[literal as usize].metadata.push(metadata(
            VALUE_RESULT,
            json!({"version":1,"kind":"non-empty"}),
        ));
        let effect = self.push(SyntaxKind::Effect, "effect", self.line(row));
        self.nodes[effect as usize].metadata.push(metadata(
            CHANGE,
            json!({"version":2,"mode":mode,"targetCapture":0,"valueCapture":1,"identityDelta":true,"targetOnly":true}),
        ));
        self.seal(effect, &[(0, variable), (1, literal)]);
        self.nodes[effect as usize]
            .metadata
            .push(metadata(OPERANDS, json!({"version":1,"prefix":[1]})));
        effect
    }
    fn read(&mut self, row: usize, spelling: &str, name: &str, local: bool, list: bool) -> u64 {
        let variable = self.var(row, spelling, name, local, list);
        let effect = self.push(SyntaxKind::Effect, "effect", self.line(row));
        self.seal(effect, &[(0, variable)]);
        self.nodes[effect as usize]
            .metadata
            .push(metadata(OPERANDS, json!({"version":1,"prefix":[0]})));
        self.nodes[effect as usize].metadata.push(metadata(
            LOCAL_EFFECTS,
            json!({"version":1,"kind":"preserve"}),
        ));
        effect
    }
    fn container(&mut self, row: usize, kind: SyntaxKind, ids: &[u64], flow: Option<&str>) -> u64 {
        let id = self.push(
            kind,
            if kind == SyntaxKind::Structure {
                "structure"
            } else {
                "section"
            },
            self.line(row),
        );
        self.nodes[id as usize].children = ids.to_vec();
        self.nodes[id as usize].captures.push(Capture {
            name: "body".into(),
            value: CaptureValue::Nodes(ids.to_vec()),
        });
        if let Some(flow) = flow {
            self.nodes[id as usize]
                .metadata
                .push(metadata(FLOW, json!({"version":1,"kind":flow})));
            self.nodes[id as usize].metadata.push(metadata(
                LOCAL_EFFECTS,
                json!({"version":1,"kind":"preserve"}),
            ));
        }
        id
    }
    fn finish(self, roots: &[u64]) -> Document {
        self.finish_with(roots, Vec::new(), Vec::new(), "file:///fixture.sk")
    }
    fn finish_with(
        self,
        roots: &[u64],
        blocked: Vec<TextRange>,
        functions: Vec<FunctionSymbol>,
        uri: &str,
    ) -> Document {
        let mapped = skript_parser::MappedSource::identity(Arc::clone(&self.source));
        let raw = skript_parser::parse_raw_tree(
            &mapped,
            skript_parser::RawTreeOptions::for_skript_version(2, 15),
        );
        Document::from_ast(
            self.source,
            uri.parse().unwrap(),
            catalog(),
            &AstTree {
                roots: roots.to_vec(),
                nodes: self.nodes,
            },
            &raw,
            blocked,
            functions,
            None,
        )
        .unwrap()
    }
}
fn number() -> InferredType {
    InferredType::Values {
        classes: BTreeSet::from(["java.lang.Number".into()]),
        multiple: false,
        maybe_unset: false,
    }
}
fn string() -> InferredType {
    InferredType::Values {
        classes: BTreeSet::from(["java.lang.String".into()]),
        multiple: false,
        maybe_unset: false,
    }
}
fn inferred(document: &Document, row: u32, column: u32) -> InferredType {
    document
        .type_at(Position::new(row, column))
        .unwrap()
        .1
        .clone()
}

#[test]
fn exact_assignments_keep_disjoint_names_and_reaching_definition() {
    let mut fixture =
        Fixture::new("owner:\n    bind {_a} = 1\n    bind {_b} = 2\n    inspect {_a}");
    let a = fixture.assign(1, "{_a}", "a", "1", "java.lang.Number", true, false, "set");
    let b = fixture.assign(2, "{_b}", "b", "2", "java.lang.Number", true, false, "set");
    let read = fixture.read(3, "{_a}", "a", true, false);
    let root = fixture.container(0, SyntaxKind::Structure, &[a, b, read], None);
    let document = fixture.finish(&[root]);
    assert_eq!(inferred(&document, 3, 13), number());
    assert_eq!(
        document
            .references_at(Position::new(3, 13), &[], false)
            .len(),
        1
    );
    assert_eq!(
        document
            .references_at(Position::new(3, 13), &[], true)
            .len(),
        2
    );
    assert_eq!(
        document.definitions_at(Position::new(3, 13), &[]),
        vec![Location::new(
            document.uri.clone(),
            Range::new(Position::new(1, 9), Position::new(1, 13))
        )]
    );
}
#[test]
fn potential_case_alias_write_invalidates_without_selecting_a_runtime_policy() {
    let mut fixture =
        Fixture::new("owner:\n    bind {_a} = 1\n    bind {_A} = 2\n    inspect {_a}");
    let a = fixture.assign(1, "{_a}", "a", "1", "java.lang.Number", true, false, "set");
    let alias = fixture.assign(2, "{_A}", "A", "2", "java.lang.Number", true, false, "set");
    let read = fixture.read(3, "{_a}", "a", true, false);
    let root = fixture.container(0, SyntaxKind::Structure, &[a, alias, read], None);
    assert_eq!(
        inferred(&fixture.finish(&[root]), 3, 13),
        InferredType::Unknown
    );
}
#[test]
fn rejected_raw_statement_and_unknown_effect_are_both_barriers() {
    for omitted in [true, false] {
        let mut fixture = Fixture::new("owner:\n    bind {_a} = 1\n    opaque\n    inspect {_a}");
        let a = fixture.assign(1, "{_a}", "a", "1", "java.lang.Number", true, false, "set");
        let mut body = vec![a];
        if !omitted {
            body.push(fixture.push(SyntaxKind::Effect, "effect", fixture.line(2)));
        }
        body.push(fixture.read(3, "{_a}", "a", true, false));
        let root = fixture.container(0, SyntaxKind::Structure, &body, None);
        assert_eq!(
            inferred(&fixture.finish(&[root]), 3, 13),
            InferredType::Unknown
        );
    }
}

#[test]
fn opaque_runtime_parents_isolate_operand_reads_before_and_after_evaluation() {
    for (kind, summary) in [
        (SyntaxKind::Effect, "effect"),
        (SyntaxKind::Condition, "condition"),
        (SyntaxKind::Section, "section"),
        (SyntaxKind::Expression, "registered-expression"),
        (SyntaxKind::Expression, "custom"),
        (SyntaxKind::Expression, "default-expression"),
        (SyntaxKind::Expression, "literal"),
        (SyntaxKind::Expression, "arithmetic"),
        (SyntaxKind::Function, "function"),
    ] {
        let mut fixture =
            Fixture::new("owner:\n    bind {_a} = 1\n    foreign {_a}\n    inspect {_a}");
        let write = fixture.assign(1, "{_a}", "a", "1", "java.lang.Number", true, false, "set");
        let operand = fixture.var(2, "{_a}", "a", true, false);
        let parent = fixture.push(kind, summary, fixture.line(2));
        fixture.seal(parent, &[(0, operand)]);
        let read = fixture.read(3, "{_a}", "a", true, false);
        let root = fixture.container(0, SyntaxKind::Structure, &[write, parent, read], None);
        let document = fixture.finish(&[root]);
        assert_eq!(
            inferred(&document, 2, 13),
            InferredType::Unknown,
            "{summary}"
        );
        assert_eq!(
            inferred(&document, 3, 13),
            InferredType::Unknown,
            "{summary}"
        );
    }
}

#[test]
fn opaque_operands_cannot_share_a_previous_operands_assignment_fact() {
    let mut fixture = Fixture::new(
        "owner:\n    bind {_a} = 1\n    invoke(bind {_a} = text, {_a})\n    inspect {_a}",
    );
    let initial = fixture.assign(1, "{_a}", "a", "1", "java.lang.Number", true, false, "set");
    let mutation = fixture.assign(
        2,
        "{_a}",
        "a",
        "text",
        "java.lang.String",
        true,
        false,
        "set",
    );
    let operand = fixture.var(2, "{_a}", "a", true, false);
    let start = fixture.line(2).start
        + fixture.source[fixture.line(2).start..fixture.line(2).end]
            .rfind("{_a}")
            .unwrap();
    fixture.nodes[operand as usize].span.virtual_range = WitRange {
        start: start as u64,
        end: (start + 4) as u64,
    };
    fixture.nodes[operand as usize].span.origins[0].original_range =
        fixture.nodes[operand as usize].span.virtual_range.clone();
    let parent = fixture.push(SyntaxKind::Expression, "custom", fixture.line(2));
    fixture.nodes[parent as usize].children = vec![mutation, operand];
    let read = fixture.read(3, "{_a}", "a", true, false);
    let root = fixture.container(0, SyntaxKind::Structure, &[initial, parent, read], None);
    let document = fixture.finish(&[root]);
    assert_eq!(
        document
            .type_at(document.lines.position(&document.source, start).unwrap())
            .map(|(_, ty)| ty),
        Some(&InferredType::Unknown)
    );
    assert_eq!(inferred(&document, 3, 13), InferredType::Unknown);
}

#[test]
fn declared_return_type_does_not_assert_a_nonempty_runtime_result() {
    let mut expression = node(
        "value",
        0,
        SyntaxKind::Expression,
        "registered-expression",
        TextRange::new(0, 5),
    );
    expression.summary.as_mut().unwrap().return_type = Some("java.lang.String".into());
    assert!(matches!(
        known_type(&expression),
        InferredType::Values {
            maybe_unset: true,
            ..
        }
    ));
    expression.metadata.push(metadata(
        VALUE_RESULT,
        json!({"version":1,"kind":"non-empty"}),
    ));
    assert!(matches!(
        known_type(&expression),
        InferredType::Values {
            maybe_unset: false,
            ..
        }
    ));
}

#[test]
fn direct_condition_evaluation_requires_a_local_preservation_witness() {
    for summary in ["condition", "none", "variable"] {
        for preserve in [false, true] {
            let mut fixture =
                Fixture::new("owner:\n    bind {_a} = 1\n    foreign {_a}\n    inspect {_a}");
            let write = fixture.assign(1, "{_a}", "a", "1", "java.lang.Number", true, false, "set");
            let operand = fixture.var(2, "{_a}", "a", true, false);
            let condition = fixture.push(SyntaxKind::Condition, "condition", fixture.line(2));
            match summary {
                "none" => fixture.nodes[condition as usize].summary = None,
                "variable" => {
                    fixture.nodes[condition as usize].summary =
                        fixture.nodes[operand as usize].summary.clone()
                }
                _ => {}
            }
            fixture.seal(condition, &[(0, operand)]);
            if preserve {
                fixture.nodes[condition as usize].metadata.push(metadata(
                    LOCAL_EFFECTS,
                    json!({"version":1,"kind":"preserve"}),
                ));
                fixture.nodes[condition as usize]
                    .metadata
                    .push(metadata(OPERANDS, json!({"version":1,"prefix":[0]})));
            }
            let read = fixture.read(3, "{_a}", "a", true, false);
            let root = fixture.container(0, SyntaxKind::Structure, &[write, condition, read], None);
            let document = fixture.finish(&[root]);
            let expected = if preserve {
                number()
            } else {
                InferredType::Unknown
            };
            assert_eq!(inferred(&document, 2, 13), expected);
            assert_eq!(inferred(&document, 3, 13), expected);
        }
    }
}

#[test]
fn unknown_branch_condition_clears_incoming_facts_before_operands_and_body() {
    let mut fixture = Fixture::new(
        "owner:\n    bind {_a} = 1\n    if foreign {_a}:\n        inspect {_a}\n    inspect {_a}",
    );
    let write = fixture.assign(1, "{_a}", "a", "1", "java.lang.Number", true, false, "set");
    let operand = fixture.var(2, "{_a}", "a", true, false);
    let condition = fixture.push(
        SyntaxKind::Condition,
        "condition",
        fixture.part(2, "foreign {_a}"),
    );
    fixture.seal(condition, &[(0, operand)]);
    let inside = fixture.read(3, "{_a}", "a", true, false);
    let section = fixture.container(2, SyntaxKind::Section, &[inside], Some("branch"));
    fixture.seal(section, &[(0, condition)]);
    let read = fixture.read(4, "{_a}", "a", true, false);
    let root = fixture.container(0, SyntaxKind::Structure, &[write, section, read], None);
    let document = fixture.finish(&[root]);
    for (row, column) in [(2, 16), (3, 17), (4, 13)] {
        assert_eq!(inferred(&document, row, column), InferredType::Unknown);
    }
}
#[test]
fn branches_join_types_but_reentrant_or_opaque_flow_is_unknown() {
    for flow in [Some("branch"), Some("loop"), None] {
        let mut fixture = Fixture::new(
            "owner:\n    bind {_a} = 1\n    branch:\n        bind {_a} = text\n    inspect {_a}",
        );
        let a = fixture.assign(1, "{_a}", "a", "1", "java.lang.Number", true, false, "set");
        let inside = fixture.assign(
            3,
            "{_a}",
            "a",
            "text",
            "java.lang.String",
            true,
            false,
            "set",
        );
        let section = fixture.container(2, SyntaxKind::Section, &[inside], flow);
        let read = fixture.read(4, "{_a}", "a", true, false);
        let root = fixture.container(0, SyntaxKind::Structure, &[a, section, read], None);
        let document = fixture.finish(&[root]);
        if flow == Some("branch") {
            assert_eq!(
                inferred(&document, 4, 13),
                InferredType::Values {
                    classes: BTreeSet::from(["java.lang.Number".into(), "java.lang.String".into()]),
                    multiple: false,
                    maybe_unset: false
                }
            );
            assert_eq!(document.definitions_at(Position::new(4, 13), &[]).len(), 2);
        } else {
            assert_eq!(inferred(&document, 4, 13), InferredType::Unknown);
        }
    }
}
#[test]
fn list_append_unions_delete_marks_unset_and_scalar_add_stays_unknown() {
    for (name, list, mode, expected) in [
        ("a", false, "add", InferredType::Unknown),
        ("a", false, "delete", InferredType::unset()),
        (
            "a::*",
            true,
            "add",
            InferredType::Values {
                classes: BTreeSet::from(["java.lang.Number".into(), "java.lang.String".into()]),
                multiple: true,
                maybe_unset: false,
            },
        ),
    ] {
        let spelling = format!("{{_{name}}}");
        let mut fixture = Fixture::new(&format!(
            "owner:\n    bind {spelling} = 1\n    mutate {spelling} = text\n    inspect {spelling}"
        ));
        let a = fixture.assign(
            1,
            &spelling,
            name,
            "1",
            "java.lang.Number",
            true,
            list,
            "set",
        );
        let b = fixture.assign(
            2,
            &spelling,
            name,
            "text",
            "java.lang.String",
            true,
            list,
            mode,
        );
        let read = fixture.read(3, &spelling, name, true, list);
        let root = fixture.container(0, SyntaxKind::Structure, &[a, b, read], None);
        assert_eq!(inferred(&fixture.finish(&[root]), 3, 13), expected);
    }
}
#[test]
fn global_definitions_link_across_current_documents_without_runtime_type_claims() {
    let mut fixture = Fixture::new("owner:\n    bind {shared} = 1");
    let write = fixture.assign(
        1,
        "{shared}",
        "shared",
        "1",
        "java.lang.Number",
        false,
        false,
        "set",
    );
    let root = fixture.container(0, SyntaxKind::Structure, &[write], None);
    let definition = fixture.finish_with(&[root], Vec::new(), Vec::new(), "file:///definition.sk");
    let mut fixture = Fixture::new("owner:\n    inspect {shared}");
    let read = fixture.read(1, "{shared}", "shared", false, false);
    let root = fixture.container(0, SyntaxKind::Structure, &[read], None);
    let document = fixture.finish(&[root]);
    assert_eq!(inferred(&document, 1, 14), InferredType::Unknown);
    let targets = document.definitions_at(Position::new(1, 14), &[&definition]);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].uri, definition.uri);
    assert!(
        document
            .definitions_at(Position::new(1, 14), &[])
            .is_empty()
    );
}
#[test]
fn functions_resolve_exact_registration_and_never_foreign_local_symbols() {
    let mut fixture = Fixture::new("owner:\n    call(1)");
    let call = fixture.push(SyntaxKind::Function, "function", fixture.part(1, "call(1)"));
    fixture.nodes[call as usize].function = Some(FunctionCall {
        name: "call".into(),
        definition_id: "document:1".into(),
        registration_id: "document:1:0".into(),
        arguments: Vec::new(),
    });
    let root = fixture.container(0, SyntaxKind::Structure, &[call], None);
    let mut symbol = FunctionSymbol {
        definition: FunctionDefinition {
            parser_id: "document.function".into(),
            name: "call".into(),
            definition_id: "document:1".into(),
            registration_id: "document:1:0".into(),
            registration_order: 0,
            return_type: None,
            return_type_is_single: true,
            parameters: Vec::new(),
            metadata: BTreeMap::new(),
        },
        range: Range::new(Position::new(0, 0), Position::new(0, 5)),
        local: false,
        name_range: Some(Range::new(Position::new(0, 0), Position::new(0, 4))),
    };
    symbol
        .definition
        .metadata
        .insert("function.document-id".into(), "native-source".into());
    symbol
        .definition
        .metadata
        .insert("function.revision".into(), "1".into());
    for (key, value) in [
        ("function.signature", symbol.definition.signature_key()),
        ("function.document-id", "native-source".into()),
        ("function.revision", "1".into()),
        (
            "function.source-witness",
            format!("{:x}", Sha256::digest(b"call:")),
        ),
    ] {
        fixture.nodes[call as usize].metadata.push(MetadataEntry {
            owner_component_id: None,
            key: key.into(),
            value,
        });
    }
    let document = fixture.finish(&[root]);
    let mut definition =
        Fixture::new("call:").finish_with(&[], Vec::new(), vec![symbol], "file:///function.sk");
    assert_eq!(
        document
            .references_at(Position::new(1, 5), &[&definition], false)
            .len(),
        1
    );
    assert_eq!(
        document
            .references_at(Position::new(1, 5), &[&definition], true)
            .len(),
        2
    );
    assert_eq!(
        definition
            .references_at(Position::new(0, 2), &[&document], false)
            .len(),
        1
    );
    assert!(
        document
            .references_at(Position::new(1, 9), &[&definition], true)
            .is_empty()
    );
    assert_eq!(
        document
            .definitions_at(Position::new(1, 5), &[&definition])
            .len(),
        1
    );
    for column in [8, 9, 10] {
        assert!(
            document
                .definitions_at(Position::new(1, column), &[&definition])
                .is_empty()
        );
    }
    definition.functions[0]
        .definition
        .metadata
        .insert("function.revision".into(), "2".into());
    assert_eq!(
        document
            .definitions_at(Position::new(1, 5), &[&definition])
            .len(),
        1,
        "unchanged source/signature permits analysis-only revision churn"
    );
    definition.functions[0].definition.name = "renamed".into();
    assert!(
        document
            .definitions_at(Position::new(1, 5), &[&definition])
            .is_empty()
    );
    definition.functions[0].definition.name = "call".into();
    definition.functions[0].local = true;
    assert!(
        document
            .definitions_at(Position::new(1, 5), &[&definition])
            .is_empty()
    );
    definition.functions[0].local = false;
    definition.functions[0].definition.registration_id = "other".into();
    assert!(
        document
            .definitions_at(Position::new(1, 5), &[&definition])
            .is_empty()
    );
}
#[test]
fn faults_bad_seals_dynamic_names_and_context_boundaries_fail_closed() {
    for failure in ["fault", "seal", "dynamic", "scope"] {
        let mut fixture = Fixture::new("owner:\n    bind {_a} = 1\n    inspect {_a}");
        let write = fixture.assign(1, "{_a}", "a", "1", "java.lang.Number", true, false, "set");
        let read = fixture.read(2, "{_a}", "a", true, false);
        let root = fixture.container(0, SyntaxKind::Structure, &[write, read], None);
        let mut blocked = Vec::new();
        match failure {
            "fault" => blocked.push(fixture.line(1)),
            "seal" => fixture.nodes[write as usize].text.push('!'),
            "dynamic" => {
                fixture.nodes[0].summary.as_mut().unwrap().public_data[0].json =
                    json!({"scope":"local","resolution":"dynamic","name":"a","list":false})
                        .to_string()
            }
            "scope" => fixture.nodes[3].syntax_context = 99,
            _ => unreachable!(),
        }
        let document = fixture.finish_with(&[root], blocked, Vec::new(), "file:///fixture.sk");
        assert_eq!(
            inferred(&document, 2, 13),
            InferredType::Unknown,
            "{failure}"
        );
    }
}
#[test]
fn utf16_locations_keep_exact_variable_source_range() {
    let mut fixture = Fixture::new("# 😀\nowner:\n    bind {_値} = 1\n    inspect {_値}");
    let write = fixture.assign(
        2,
        "{_値}",
        "値",
        "1",
        "java.lang.Number",
        true,
        false,
        "set",
    );
    let read = fixture.read(3, "{_値}", "値", true, false);
    let root = fixture.container(1, SyntaxKind::Structure, &[write, read], None);
    let document = fixture.finish(&[root]);
    assert_eq!(inferred(&document, 3, 13), number());
    assert_eq!(
        document.definitions_at(Position::new(3, 13), &[])[0].range,
        Range::new(Position::new(2, 9), Position::new(2, 13))
    );
    assert!(document.type_at(Position::new(0, 3)).is_none());
}
#[test]
fn parameter_contract_seeds_declared_type_and_exact_definition_span() {
    for malformed in [false, true] {
        let mut fixture = Fixture::new("function value: number:\n    inspect {_value}");
        let read = fixture.read(1, "{_value}", "value", true, false);
        let root = fixture.container(0, SyntaxKind::Structure, &[read], None);
        fixture.nodes[root as usize].metadata.push(metadata(PARAMETERS,json!({"version":1,"parameters":[{
            "identity":{"scope":"local","resolution":"static","name":"value","aliasKey":"value","list":false,"casePolicy":"unknown"},
            "className":"java.lang.Number","single":true,"start":9,"end":if malformed {999} else {14}
        }]})));
        let document = fixture.finish(&[root]);
        assert_eq!(
            inferred(&document, 1, 13),
            if malformed {
                InferredType::Unknown
            } else {
                InferredType::Values {
                    classes: BTreeSet::from(["java.lang.Number".into()]),
                    multiple: false,
                    maybe_unset: true,
                }
            }
        );
        if !malformed {
            assert_eq!(
                document.definitions_at(Position::new(1, 13), &[])[0].range,
                Range::new(Position::new(0, 9), Position::new(0, 14))
            );
        }
    }
}
#[test]
fn native_capture_cannot_be_retargeted_to_another_source_line() {
    let mut fixture = Fixture::new("owner:\n    bind {_a} = 1\n    inspect {_b}");
    let write = fixture.assign(1, "{_a}", "a", "1", "java.lang.Number", true, false, "set");
    let read = fixture.read(2, "{_b}", "b", true, false);
    let foreign = 3;
    fixture.nodes[write as usize]
        .captures
        .iter_mut()
        .find(|capture| capture.name.starts_with("0:"))
        .unwrap()
        .value = CaptureValue::Node(foreign);
    fixture.nodes[write as usize].children.push(foreign);
    let root = fixture.container(0, SyntaxKind::Structure, &[write, read], None);
    let document = fixture.finish(&[root]);
    assert_eq!(inferred(&document, 2, 13), InferredType::Unknown);
    assert!(
        document
            .definitions_at(Position::new(2, 13), &[])
            .is_empty()
    );
}
#[test]
fn list_append_after_unknown_state_never_assumes_an_empty_previous_list() {
    for initial in [false, true] {
        let mut fixture = Fixture::new(
            "owner:\n    bind {_xs::*} = 1\n    opaque\n    append {_xs::*} = text\n    inspect {_xs::*}",
        );
        let mut body = Vec::new();
        if initial {
            body.push(fixture.assign(
                1,
                "{_xs::*}",
                "xs::*",
                "1",
                "java.lang.Number",
                true,
                true,
                "set",
            ));
        }
        body.push(fixture.push(SyntaxKind::Effect, "effect", fixture.line(2)));
        body.push(fixture.assign(
            3,
            "{_xs::*}",
            "xs::*",
            "text",
            "java.lang.String",
            true,
            true,
            "add",
        ));
        body.push(fixture.read(4, "{_xs::*}", "xs::*", true, true));
        let root = fixture.container(0, SyntaxKind::Structure, &body, None);
        assert_eq!(
            inferred(&fixture.finish(&[root]), 4, 13),
            InferredType::Unknown
        );
    }
}
#[test]
fn reordered_macro_body_does_not_restore_source_execution_order() {
    let mut fixture =
        Fixture::new("owner:\n    bind {_a} = 1\n    bind {_a} = text\n    inspect {_a}");
    let first = fixture.assign(1, "{_a}", "a", "1", "java.lang.Number", true, false, "set");
    let second = fixture.assign(
        2,
        "{_a}",
        "a",
        "text",
        "java.lang.String",
        true,
        false,
        "set",
    );
    let read = fixture.read(3, "{_a}", "a", true, false);
    let root = fixture.container(0, SyntaxKind::Structure, &[second, first, read], None);
    assert_eq!(
        inferred(&fixture.finish(&[root]), 3, 13),
        InferredType::Unknown
    );
}
#[test]
fn unicode_uncertain_aliases_do_not_create_unrelated_definition_targets() {
    let mut fixture = Fixture::new("owner:\n    bind {_hello} = 1\n    inspect {_日本}");
    let write = fixture.assign(
        1,
        "{_hello}",
        "hello",
        "1",
        "java.lang.Number",
        true,
        false,
        "set",
    );
    let read = fixture.read(2, "{_日本}", "日本", true, false);
    let root = fixture.container(0, SyntaxKind::Structure, &[write, read], None);
    let document = fixture.finish(&[root]);
    assert!(
        document
            .definitions_at(Position::new(2, 13), &[])
            .is_empty()
    );
}

#[test]
fn uncertain_cardinality_never_becomes_a_scalar_fact() {
    let source = "value";
    let mut expression = node(
        source,
        0,
        SyntaxKind::Expression,
        "literal",
        TextRange::new(0, source.len()),
    );
    let summary = expression.summary.as_mut().unwrap();
    summary.return_type = Some("java.lang.Number".into());
    summary.multiplicity = Some(DynamicMultiplicity::Both);
    assert_eq!(known_type(&expression), InferredType::Unknown);
    assert_eq!(
        native_cardinality(number(), &expression),
        InferredType::Unknown
    );
    expression.summary.as_mut().unwrap().multiplicity = Some(DynamicMultiplicity::Multiple);
    assert!(matches!(
        native_cardinality(number(), &expression),
        InferredType::Values { multiple: true, .. }
    ));
    expression.summary.as_mut().unwrap().multiplicity = Some(DynamicMultiplicity::Single);
    assert_eq!(native_cardinality(number(), &expression), number());
}

#[test]
fn rewritten_final_forest_cannot_export_pre_macro_function_registrations() {
    let catalog = catalog();
    let mut host = crate::new_parser_host(parser_wasm::HostConfig {
        syntax_catalog: Some(Arc::clone(&catalog)),
        ..Default::default()
    })
    .unwrap();
    let source: Arc<str> = Arc::from("function project_value() :: number:\n    return 1\n");
    let mut result = host
        .parse_document(
            parser_wasm::DocumentParseRequest::new(
                "file:///project",
                "file:///project/definition.sk",
                1,
                source.as_ref(),
            ),
            Default::default(),
        )
        .unwrap();
    let uri: Uri = "file:///project/definition.sk".parse().unwrap();
    assert_eq!(result.functions.registrations().len(), 1);
    assert_eq!(
        Document::from_result(
            &result,
            Arc::clone(&source),
            uri.clone(),
            Arc::clone(&catalog)
        )
        .unwrap()
        .functions
        .len(),
        1
    );
    result.ast.roots.clear();
    assert_eq!(result.functions.registrations().len(), 1);
    assert!(
        Document::from_result(&result, source, uri, catalog)
            .unwrap()
            .functions
            .is_empty()
    );
}

#[test]
fn header_relative_parameter_contract_survives_utf8_prefix_and_rejects_bad_ranges() {
    for malformed in [false, true] {
        let mut fixture = Fixture::new(
            "# \u{65e5}\u{672c}\u{8a9e}\u{1f98a}\nfunction value: number:\n    inspect {_value}",
        );
        let read = fixture.read(2, "{_value}", "value", true, false);
        let root = fixture.container(1, SyntaxKind::Structure, &[read], None);
        fixture.nodes[root as usize].metadata.push(metadata(PARAMETERS, json!({"version":2,"parameters":[{
            "identity":{"scope":"local","resolution":"static","name":"value","aliasKey":"value","list":false,"casePolicy":"unknown"},
            "className":"java.lang.Number","single":true,"start":9,"end":if malformed {999} else {14}
        }]})));
        let document = fixture.finish(&[root]);
        if malformed {
            assert_eq!(inferred(&document, 2, 13), InferredType::Unknown);
        } else {
            assert_eq!(
                document.definitions_at(Position::new(2, 13), &[])[0].range,
                Range::new(Position::new(1, 9), Position::new(1, 14))
            );
        }
    }
}

#[test]
fn intrinsic_evaluation_steps_preserve_only_proven_formatter_boundaries() {
    for (safe_formatter, malformed) in [(true, false), (false, false), (true, true)] {
        let mut fixture = Fixture::new(
            "owner:\n    bind {_a} = text\n    bind {_b} = other\n    inspect \"%{_a}% %{_b}%\"",
        );
        let a = fixture.assign(
            1,
            "{_a}",
            "a",
            "text",
            "java.lang.String",
            true,
            false,
            "set",
        );
        let b = fixture.assign(
            2,
            "{_b}",
            "b",
            "other",
            "java.lang.String",
            true,
            false,
            "set",
        );
        let first = fixture.var(3, "{_a}", "a", true, false);
        let second = fixture.var(3, "{_b}", "b", true, false);
        let value = fixture.push(
            SyntaxKind::Expression,
            "literal",
            fixture.part(3, "\"%{_a}% %{_b}%\""),
        );
        fixture.nodes[value as usize].children = vec![first, second];
        fixture.nodes[value as usize].metadata.push(metadata(OPERANDS,json!({"version":2,"complete":true,"steps":[
            {"childIndex":if malformed {1} else {0},"preserveTypes":if safe_formatter {vec!["java.lang.String"]} else {vec![]}},
            {"childIndex":1,"preserveTypes":["java.lang.String"]}]})));
        let inspect = fixture.push(SyntaxKind::Effect, "effect", fixture.line(3));
        fixture.seal(inspect, &[(0, value)]);
        fixture.nodes[inspect as usize]
            .metadata
            .push(metadata(OPERANDS, json!({"version":1,"prefix":[0]})));
        let root = fixture.container(0, SyntaxKind::Structure, &[a, b, inspect], None);
        let document = fixture.finish(&[root]);
        let first_pos = document
            .source
            .lines()
            .nth(3)
            .unwrap()
            .find("{_a}")
            .unwrap()
            + 2;
        let second_pos = document
            .source
            .lines()
            .nth(3)
            .unwrap()
            .find("{_b}")
            .unwrap()
            + 2;
        assert_eq!(
            inferred(&document, 3, first_pos as u32),
            if malformed {
                InferredType::Unknown
            } else {
                string()
            }
        );
        assert_eq!(
            inferred(&document, 3, second_pos as u32),
            if safe_formatter && !malformed {
                string()
            } else {
                InferredType::Unknown
            }
        );
    }
}
#[test]
fn scoped_return_entry_requires_current_owner_and_cannot_revive_after_opaque_statement() {
    for (opaque, foreign, preserve) in [
        (false, false, true),
        (true, false, true),
        (false, true, true),
        (false, false, false),
    ] {
        let mut fixture = Fixture::new(if opaque {
            "owner:\n    opaque\n    bind {_a} = text\n    inspect {_a}"
        } else {
            "owner:\n    # harmless\n    bind {_a} = text\n    inspect {_a}"
        });
        let mut body = Vec::new();
        if opaque {
            body.push(fixture.push(SyntaxKind::Effect, "effect", fixture.line(1)));
        }
        let assign = fixture.assign(
            2,
            "{_a}",
            "a",
            "text",
            "java.lang.String",
            true,
            false,
            "set",
        );
        let read = fixture.read(3, "{_a}", "a", true, false);
        for id in 0..fixture.nodes.len() {
            fixture.nodes[id].metadata.push(metadata(
                EVALUATION_CONTEXT,
                json!({"version":1,"kind":"preserve"}),
            ));
        }
        if opaque {
            fixture.nodes[body[0] as usize]
                .metadata
                .retain(|entry| entry.key != EVALUATION_CONTEXT);
        }
        if !preserve {
            fixture.nodes[assign as usize]
                .metadata
                .retain(|entry| entry.key != EVALUATION_CONTEXT);
        }
        let operands = fixture.nodes[read as usize]
            .metadata
            .iter_mut()
            .find(|entry| entry.key == OPERANDS)
            .unwrap();
        operands.value=json!({"version":1,"prefix":[0],"requiredContext":{"key":"test-handler","value":"value-first"}}).to_string();
        if foreign {
            operands.owner_component_id = Some("foreign".into());
        }
        body.extend([assign, read]);
        let root = fixture.container(0, SyntaxKind::Structure, &body, None);
        fixture.nodes[root as usize].metadata.push(metadata(
            EVALUATION_CONTEXT,
            json!({"version":1,"kind":"scope","key":"test-handler","value":"value-first"}),
        ));
        assert_eq!(
            inferred(&fixture.finish(&[root]), 3, 15),
            if !opaque && !foreign && preserve {
                string()
            } else {
                InferredType::Unknown
            }
        );
    }
}

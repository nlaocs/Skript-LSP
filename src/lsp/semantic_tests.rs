use super::*;
use parser_wasm::bindings::nlaocs::skript_parser_addon::types::{
    AstContextOrigin, Capture, DynamicMultiplicity, ExpressionPossibleReturnTypesState,
    ExpressionPublicData, FunctionCall, MetadataEntry, OriginKind, ParseSummary, SourceOrigin,
    TextRange as WitRange,
};
use serde_json::json;

use crate::lsp::test_support::multi_addon_catalog as catalog;
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

#[path="semantic/transfer_tests.rs"]
mod transfer_tests;
#[path="semantic/shared_flow_tests.rs"]
mod shared_flow_tests;

#[test]
fn post_storage_type_is_independent_from_rhs_type_and_other_binding_effects() {
    for (preserves, maybe_unset) in [(false, false), (false, true), (true, true)] {
        let mut fixture = Fixture::new(
            "owner:\n    bind {_old} = 1\n    bind {_x} = source\n    inspect {_old}\n    inspect {_x}",
        );
        let first = fixture.assign(
            1,
            "{_old}",
            "old",
            "1",
            "java.lang.Number",
            true,
            false,
            "set",
        );
        let second = fixture.assign(
            2,
            "{_x}",
            "x",
            "source",
            "fixture.Input",
            true,
            false,
            "set",
        );
        fixture.nodes[second as usize]
            .metadata
            .retain(|entry| entry.key != CHANGE);
        fixture.nodes[second as usize].metadata.push(metadata(CHANGE, json!({
            "version":3,"mode":"set","targetCapture":0,"valueCapture":1,
            "targetOnly":preserves,"identityDelta":false,
            "storageResult":{"version":1,"scope":"local-scalar","types":["java.lang.String"],
                "complete":true,"multiple":false,"maybeUnset":maybe_unset,"preservesLocalBindings":preserves}
        })));
        let old = fixture.read(3, "{_old}", "old", true, false);
        let read = fixture.read(4, "{_x}", "x", true, false);
        let root = fixture.container(0, SyntaxKind::Structure, &[first, second, old, read], None);
        let document = fixture.finish(&[root]);
        let InferredType::Values {
            classes,
            maybe_unset: actual_unset,
            support,
            ..
        } = document.type_at(Position::new(2, 11)).unwrap().1.clone()
        else {
            panic!("stored type lost");
        };
        assert_eq!(classes, BTreeSet::from(["java.lang.String".into()]));
        assert_eq!(actual_unset, maybe_unset);
        assert!(support.evidence.contains(&Evidence::StorageResult));
        assert_eq!(
            inferred(&document, 3, 13).proven_types().is_some(),
            preserves
        );
        let hints = document.write_type_hints();
        assert_eq!(hints.len(), 2, "read occurrences add no hints");
        assert!(
            matches!(&hints[1].label, lsp_types::InlayHintLabel::String(label) if label == if maybe_unset { ": string?" } else { ": string" })
        );
        let hover = document.hover(Position::new(2, 11)).unwrap().1;
        assert!(
            hover.contains("provider-certified post-storage type"),
            "{hover}"
        );
    }
}

#[test]
fn unverified_storage_metadata_cannot_turn_rhs_or_history_into_current_guarantees() {
    let valid = json!({"version":1,"scope":"local-scalar","types":["java.lang.String"],"complete":true,"multiple":false,"maybeUnset":true,"preservesLocalBindings":false});
    for mutation in [
        "missing",
        "version",
        "scope",
        "complete",
        "multiple",
        "types",
        "duplicate",
        "foreign-owner",
        "old-contract",
    ] {
        let mut fixture =
            Fixture::new("owner:\n    bind {_x} = 1\n    bind {_x} = source\n    inspect {_x}");
        let first = fixture.assign(1, "{_x}", "x", "1", "java.lang.Number", true, false, "set");
        let second = fixture.assign(
            2,
            "{_x}",
            "x",
            "source",
            "java.lang.String",
            true,
            false,
            "set",
        );
        let mut result = valid.clone();
        match mutation {
            "missing" => result = serde_json::Value::Null,
            "version" => result["version"] = json!(2),
            "scope" => result["scope"] = json!("global"),
            "complete" => result["complete"] = json!(false),
            "multiple" => result["multiple"] = json!(true),
            "types" => result["types"] = json!(["java.lang.Object"]),
            _ => {}
        }
        fixture.nodes[second as usize]
            .metadata
            .retain(|entry| entry.key != CHANGE);
        let mut contract = metadata(
            CHANGE,
            json!({"version":if mutation=="old-contract" {2} else {3},"mode":"set","targetCapture":0,"valueCapture":1,"storageResult":result}),
        );
        if mutation == "foreign-owner" {
            contract.owner_component_id = Some("unrelated.provider".into());
        }
        fixture.nodes[second as usize]
            .metadata
            .push(contract.clone());
        if mutation == "duplicate" {
            fixture.nodes[second as usize].metadata.push(contract);
        }
        // This fixture deliberately enters below production from_result's
        // provider-witness boundary. Apply that same owner filter explicitly.
        retain_semantic_contracts(
            &mut fixture.nodes[second as usize],
            Some("fixture.semantic-provider"),
        );
        let read = fixture.read(3, "{_x}", "x", true, false);
        let root = fixture.container(0, SyntaxKind::Structure, &[first, second, read], None);
        let document = fixture.finish(&[root]);
        assert_eq!(
            inferred(&document, 3, 13),
            InferredType::Unknown,
            "{mutation}"
        );
        let hints = document.write_type_hints();
        if matches!(mutation, "foreign-owner" | "duplicate") {
            // Without an accepted write-role contract the renderer cannot
            // invent a binding target, even to attach an Unknown hint to it.
            assert_eq!(hints.len(), 1, "{mutation}");
        } else {
            assert!(
                matches!(&hints[1].label, lsp_types::InlayHintLabel::String(label) if label == ": ?"),
                "{mutation}"
            );
        }
    }
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
            None,
            HashSet::new(),
            Vec::new(),
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
        support: TypeSupport::inferred(),
    }
}
fn string() -> InferredType {
    InferredType::Values {
        classes: BTreeSet::from(["java.lang.String".into()]),
        multiple: false,
        maybe_unset: false,
        support: TypeSupport::inferred(),
    }
}
fn inferred(document: &Document, row: u32, column: u32) -> InferredType {
    let mut ty = document
        .type_at(Position::new(row, column))
        .unwrap()
        .1
        .clone();
    // These existing assertions compare type knowledge; provenance is asserted
    // separately below. Do not normalize certainty or candidate completeness.
    if let InferredType::Values { support, .. } = &mut ty {
        support.evidence = TypeSupport::inferred().evidence;
    }
    ty
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
                    maybe_unset: false,
                    support: TypeSupport::inferred(),
                }
            );
            assert_eq!(document.definitions_at(Position::new(4, 13), &[]).len(), 2);
        } else {
            assert_eq!(inferred(&document, 4, 13), InferredType::Unknown);
            assert!(document.definitions_at(Position::new(4, 13), &[]).is_empty());
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
                support: TypeSupport::inferred(),
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
        body: None,
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
                    support: TypeSupport::inferred(),
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
fn invalidated_local_definition_uses_only_an_unambiguous_exact_declaration() {
    for mutation in ["none", "alias", "scope", "context", "ambiguous"] {
        let mut fixture = Fixture::new(
            "function value: number:\n    opaque\n    inspect {_value}\n    bind {_value} = 1",
        );
        let opaque = fixture.push(SyntaxKind::Effect, "effect", fixture.line(1));
        let read = fixture.read(2, "{_value}", "value", true, false);
        let later = fixture.assign(
            3, "{_value}", "value", "1", "java.lang.Number", true, false, "set",
        );
        let later_range = fixture.part(3, "{_value}");
        let root = fixture.container(0, SyntaxKind::Structure, &[opaque, read, later], None);
        fixture.nodes[root as usize].metadata.push(metadata(PARAMETERS, json!({
            "version":2,"parameters":[{
                "identity":{"scope":"local","resolution":"static","name":"value","aliasKey":"value","list":false,"casePolicy":"unknown"},
                "className":"java.lang.Number","single":true,"start":9,"end":14
            }]
        })));
        let mut document = fixture.finish(&[root]);
        let declaration = document
            .writes
            .iter_mut()
            .find(|write| write.role == WriteRole::ExplicitDeclaration)
            .unwrap();
        match mutation {
            "alias" => declaration.identity.key.name = "VALUE".into(),
            "scope" => declaration.identity.key.scope += 1,
            "context" => declaration.identity.key.context += 1,
            "ambiguous" => {
                let mut other = declaration.clone();
                other.range = later_range;
                document.writes.push(other);
            }
            _ => {}
        }
        let position = Position::new(2, 13);
        assert_eq!(inferred(&document, 2, 13), InferredType::Unknown);
        let expected = if mutation == "none" {
            vec![Location::new(
                document.uri.clone(),
                Range::new(Position::new(0, 9), Position::new(0, 14)),
            )]
        } else {
            Vec::new()
        };
        assert_eq!(document.definitions_at(position, &[]), expected, "{mutation}");
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

#[test]
fn call_hierarchy_body_membership_excludes_header_children_and_ambiguous_owners() {
    let mut fixture = Fixture::new("owner:\n    call()\n    call()");
    let header = fixture.push(SyntaxKind::Function, "function", fixture.part(1, "call()"));
    let body = fixture.push(SyntaxKind::Function, "function", fixture.part(2, "call()"));
    for id in [header, body] {
        fixture.nodes[id as usize].function = Some(FunctionCall {
            name: "call".into(),
            definition_id: "callee".into(),
            registration_id: "callee:0".into(),
            arguments: Vec::new(),
        });
    }
    let root = fixture.container(0, SyntaxKind::Structure, &[body], None);
    fixture.nodes[root as usize].children.push(header);
    let mapped = skript_parser::MappedSource::identity(Arc::clone(&fixture.source));
    let raw = skript_parser::parse_raw_tree(
        &mapped,
        skript_parser::RawTreeOptions::for_skript_version(2, 15),
    );
    let ast = AstTree {
        roots: vec![root],
        nodes: fixture.nodes.clone(),
    };
    let lines = LineIndex::new(&fixture.source);
    let symbol = FunctionSymbol {
        definition: FunctionDefinition {
            parser_id: "document.function".into(),
            name: "owner".into(),
            definition_id: "owner".into(),
            registration_id: "owner:0".into(),
            registration_order: 0,
            return_type: None,
            return_type_is_single: true,
            parameters: Vec::new(),
            metadata: BTreeMap::new(),
        },
        range: lines.range(&fixture.source, fixture.line(0)).unwrap(),
        local: false,
        name_range: None,
        body: call_hierarchy::body(&fixture.nodes[root as usize], &raw, &lines, &fixture.source),
    };
    assert!(symbol.body.is_some());
    let mut document = fixture.finish_with(
        &[root],
        Vec::new(),
        vec![symbol.clone()],
        "file:///body-membership.sk",
    );
    assert_eq!(
        document.occurrences.len(),
        2,
        "both accepted call occurrences are indexed by shared navigation"
    );
    document.index_function_calls(&ast);
    assert_eq!(
        document.calls.len(),
        1,
        "only the native body capture owns runtime body edges"
    );
    document.functions.push(symbol);
    document.index_function_calls(&ast);
    assert!(
        document.calls.is_empty(),
        "a node shared by two declarations cannot invent an enclosing caller"
    );
}

#[test]
fn call_hierarchy_distinguishes_provider_overloads_at_the_same_source_location() {
    let mut fixture = Fixture::new("caller:\n    over(1)\n    over(\"x\")\nover:");
    let first = fixture.push(SyntaxKind::Function, "function", fixture.part(1, "over(1)"));
    let second = fixture.push(
        SyntaxKind::Function,
        "function",
        fixture.part(2, "over(\"x\")"),
    );
    let caller = fixture.container(0, SyntaxKind::Structure, &[first, second], None);
    let callee = fixture.container(3, SyntaxKind::Structure, &[], None);
    let mapped = skript_parser::MappedSource::identity(Arc::clone(&fixture.source));
    let raw = skript_parser::parse_raw_tree(
        &mapped,
        skript_parser::RawTreeOptions::for_skript_version(2, 15),
    );
    let lines = LineIndex::new(&fixture.source);
    let make = |name: &str, class: Option<&str>, node: u64, row: usize| {
        let definition = FunctionDefinition {
            parser_id: "document.function".into(),
            name: name.into(),
            definition_id: format!("{name}:{class:?}"),
            registration_id: format!("{name}:{class:?}:0"),
            registration_order: 0,
            return_type: None,
            return_type_is_single: true,
            parameters: class
                .map(|class| {
                    vec![skript_parser::FunctionParameterDefinition {
                        name: "value".into(),
                        parameter_type: syntaxes::ClassName(class.into()),
                        single: true,
                        optional: false,
                    }]
                })
                .unwrap_or_default(),
            metadata: BTreeMap::from([
                ("function.document-id".into(), "fixture-owner".into()),
                ("function.revision".into(), "1".into()),
            ]),
        };
        FunctionSymbol {
            definition,
            range: lines.range(&fixture.source, fixture.line(row)).unwrap(),
            local: false,
            name_range: Some(
                lines
                    .range(&fixture.source, fixture.part(row, name))
                    .unwrap(),
            ),
            body: call_hierarchy::body(
                &fixture.nodes[node as usize],
                &raw,
                &lines,
                &fixture.source,
            ),
        }
    };
    let owner = make("caller", None, caller, 0);
    let a = make("over", Some("java.lang.Number"), callee, 3);
    let b = make("over", Some("java.lang.String"), callee, 3);
    for (id, symbol) in [(first, &a), (second, &b)] {
        fixture.nodes[id as usize].function = Some(FunctionCall {
            name: "over".into(),
            definition_id: symbol.definition.definition_id.clone(),
            registration_id: symbol.definition.registration_id.clone(),
            arguments: Vec::new(),
        });
        for (key, value) in [
            ("function.signature", symbol.definition.signature_key()),
            ("function.document-id", "fixture-owner".into()),
            ("function.revision", "1".into()),
            (
                "function.source-witness",
                format!("{:x}", Sha256::digest(fixture.source.as_bytes())),
            ),
        ] {
            fixture.nodes[id as usize].metadata.push(MetadataEntry {
                owner_component_id: None,
                key: key.into(),
                value,
            });
        }
    }
    let ast = AstTree {
        roots: vec![caller, callee],
        nodes: fixture.nodes.clone(),
    };
    let mut document = fixture.finish_with(
        &[caller, callee],
        Vec::new(),
        vec![owner, a, b],
        "file:///overload-provider.sk",
    );
    document.index_function_calls(&ast);
    let owner = document
        .prepare_hierarchy(Position::new(0, 1), &[], "session")
        .unwrap()
        .remove(0);
    let outgoing = document.hierarchy_outgoing(&owner, &[], "session").unwrap();
    assert_eq!(
        outgoing.len(),
        2,
        "same source position must not merge distinct registered function signatures"
    );
    assert_eq!(outgoing[0].to.uri, outgoing[1].to.uri);
    assert_eq!(
        outgoing[0].to.selection_range,
        outgoing[1].to.selection_range
    );
    assert_ne!(outgoing[0].to.data, outgoing[1].to.data);
    for call in outgoing {
        assert_eq!(call.from_ranges.len(), 1);
        let incoming = document
            .hierarchy_incoming(&call.to, &[], "session")
            .unwrap();
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].from, owner);
        assert_eq!(incoming[0].from_ranges, call.from_ranges);
    }
}

#[test]
fn document_symbols_option_expansion_keeps_original_outline_geometry() {
    let catalog = crate::lsp::test_support::runtime_catalog();
    let mut host = crate::new_parser_host(parser_wasm::HostConfig {
        syntax_catalog: Some(Arc::clone(&catalog)),
        runtime_profile: parser_wasm::RuntimeProfile {
            skript_version: Some("2.15.4".into()),
            ..Default::default()
        },
        ..Default::default()
    }).unwrap();
    let source: Arc<str> = Arc::from("# \u{65e5}\u{1f98a}\noptions:\n    value: 1 day\nfunction label() :: text:\n    return \"ready\"\non join:\n    set {_remaining} to {@value} - {_elapsed}\n");
    let uri: Uri = "file:///mapped-outline/public.sk".parse().unwrap();
    let result = host.parse_document(
        parser_wasm::DocumentParseRequest::new("file:///mapped-outline", uri.as_str(), 1, source.as_ref()),
        Default::default(),
    ).unwrap();
    assert_eq!(result.execution_status, parser_wasm::DocumentExecutionStatus::Complete);
    assert!(result.component_failures.is_empty(), "{:?}", result.component_failures);
    let semantic = Document::from_result(&result, Arc::clone(&source), uri, Arc::clone(&catalog)).unwrap();
    assert_eq!(semantic.symbols.len(), 3);
    assert_eq!(semantic.symbols[1].name, "label");
    assert_eq!(semantic.symbols[1].selection_range, Range::new(Position::new(3,9), Position::new(3,14)));
    assert_eq!(semantic.symbols[1].range, Range::new(Position::new(3,0), Position::new(4,18)));
    assert_eq!(semantic.symbols[2].range.end, Position::new(6,source.lines().nth(6).unwrap().len() as u32));
}

#[test]
fn option_expansion_keeps_exact_binding_hovers_and_write_hints() {
    for (value, rhs, expected) in [("3", "{@value}", ": long"), ("1 day", "{@value} - {_elapsed}", ": ?")] {
        let catalog = crate::lsp::test_support::runtime_catalog();
        let mut host = crate::new_parser_host(parser_wasm::HostConfig {
            syntax_catalog: Some(Arc::clone(&catalog)),
            runtime_profile: parser_wasm::RuntimeProfile { skript_version:Some("2.15.4".into()), ..Default::default() },
            ..Default::default()
        }).unwrap();
        let source: Arc<str> = Arc::from(format!("# \u{65e5}\u{1f98a}\r\noptions:\r\n    value: {value}\r\non join:\r\n    set {{_remaining}} to {rhs}\r\n"));
        let uri: Uri="file:///option-provenance/public.sk".parse().unwrap();
        let result=host.parse_document(parser_wasm::DocumentParseRequest::new(
            "file:///option-provenance",uri.as_str(),1,source.as_ref(),
        ),Default::default()).unwrap();
        assert_eq!(result.execution_status,parser_wasm::DocumentExecutionStatus::Complete);
        assert!(result.component_failures.is_empty(),"{:?}",result.component_failures);
        let semantic=Document::from_result(&result,Arc::clone(&source),uri,Arc::clone(&catalog)).unwrap();
        let hint=semantic.write_type_hints().into_iter().find(|h|h.position.line==4).expect("exact retained target has a write hint");
        assert_eq!(hint.position,Position::new(4,20));
        assert!(matches!(&hint.label,lsp_types::InlayHintLabel::String(label) if label==expected), "{hint:?}");
        let target=Position::new(4,9);
        assert!(semantic.hover_fields(target).is_some());
        let hover=crate::lsp::hover::Document::from_result(
            result,Arc::clone(&source),catalog,lsp_types::MarkupKind::Markdown,
        ).unwrap();
        let shown=hover.at(target).expect("source-backed target hover");
        assert_eq!(shown.range,Some(Range::new(Position::new(4,8),Position::new(4,20))));
        assert!(hover.at(Position::new(4,25)).is_none(),"expanded literal must not gain an exact original hover");
        if rhs.contains("{_elapsed}") {
            let line=source.lines().nth(4).unwrap();
            let col=line.find("{_elapsed}").unwrap()+1;
            assert!(hover.at(Position::new(4,col as u32)).is_some(),"retained RHS variable survives beside the expansion");
        }
    }
}

#[test]
fn document_symbols_require_the_current_sealed_presentation_record() {
    let catalog=crate::lsp::test_support::runtime_catalog();
    let mut host=crate::new_parser_host(parser_wasm::HostConfig {
        syntax_catalog:Some(Arc::clone(&catalog)),
        runtime_profile:parser_wasm::RuntimeProfile {skript_version:Some("2.15.4".into()),..Default::default()},
        ..Default::default()
    }).unwrap();
    let source:Arc<str>=Arc::from("# \u{1f98a}\non join:\n    set {_text} to \"hello\"\n");
    let uri:Uri="file:///sealed-outline/public.sk".parse().unwrap();
    let mut result=host.parse_document(parser_wasm::DocumentParseRequest::new(
        "file:///sealed-outline",uri.as_str(),1,source.as_ref()),Default::default()).unwrap();
    let original=result.ast.clone();
    assert_eq!(Document::from_result(&result,Arc::clone(&source),uri.clone(),Arc::clone(&catalog)).unwrap().symbols.len(),1);
    for change in 0..3 {
        result.ast=original.clone();
        let node=result.ast.nodes.iter_mut().find(|node|node.metadata.iter().any(|entry|entry.key==parser_wasm::DOCUMENT_SYMBOL_METADATA)).unwrap();
        let entry=node.metadata.iter_mut().find(|entry|entry.key==parser_wasm::DOCUMENT_SYMBOL_METADATA).unwrap();
        match change {
            0=>entry.owner_component_id=Some("unaccepted-provider".into()),
            1=>entry.value=entry.value.replace("on join","invented declaration"),
            _=>node.span.virtual_range.end+=1,
        }
        assert!(Document::from_result(&result,Arc::clone(&source),uri.clone(),Arc::clone(&catalog))
            .unwrap().symbols.is_empty(),"changed sealed presentation record: {change}");
    }
}

#[test]
fn document_symbols_use_native_hierarchy_exact_utf16_ranges_and_owned_records() {
    let mut fixture = Fixture::new("# 🌳\n親:\n    子:\n        value\nnext:");
    let child = fixture.push(SyntaxKind::Section, "section", fixture.line(2));
    let root = fixture.push(SyntaxKind::Structure, "structure", fixture.line(1));
    let next = fixture.push(SyntaxKind::Structure, "structure", fixture.line(4));
    fixture.nodes[root as usize].children.push(child);
    for (id, name) in [(root, "親"), (child, "子"), (next, "next")] {
        fixture.nodes[id as usize].metadata.push(metadata(parser_wasm::DOCUMENT_SYMBOL_METADATA,
            json!({"version":1,"name":name,"kind":3,"detail":null,"selection":{"start":0,"end":name.len()}})));
    }
    let mapped = skript_parser::MappedSource::identity(Arc::clone(&fixture.source));
    let raw = skript_parser::parse_raw_tree(
        &mapped,
        skript_parser::RawTreeOptions::for_skript_version(2, 15),
    );
    let lines = LineIndex::new(&fixture.source);
    let mut ast = AstTree {
        roots: vec![next, root],
        nodes: fixture.nodes,
    };
    let collect = |ast: &AstTree| {
        document_symbols::from_ast(ast, &raw, &mapped, &lines, &[], |_, entry| {
            entry.owner_component_id.as_deref() == Some("fixture.semantic-provider")
        })
    };
    let symbols = collect(&ast);
    assert_eq!(
        symbols.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["親", "next"]
    );
    assert_eq!(
        symbols[0].range,
        Range::new(Position::new(1, 0), Position::new(3, 13))
    );
    assert_eq!(
        symbols[0].selection_range,
        Range::new(Position::new(1, 0), Position::new(1, 1))
    );
    assert_eq!(symbols[0].children.as_ref().unwrap()[0].name, "子");
    let mut altered_raw = raw.clone();
    altered_raw
        .nodes
        .iter_mut()
        .find(|node| {
            node.code_span.as_ref().is_some_and(|span| {
                span.virtual_range.start
                    == ast.nodes[child as usize].span.virtual_range.start as usize
            })
        })
        .unwrap()
        .parent = None;
    let altered = document_symbols::from_ast(&ast, &altered_raw, &mapped, &lines, &[], |_, _| true);
    assert!(
        altered[0].children.is_none(),
        "contained geometry without native raw ancestry cannot make a child"
    );
    let blocked = document_symbols::from_ast(
        &ast,
        &raw,
        &mapped,
        &lines,
        &[(
            ast.nodes[root as usize].span.virtual_range.start,
            ast.nodes[root as usize].span.virtual_range.end,
        )],
        |_, _| true,
    );
    assert!(!blocked.iter().any(|symbol| symbol.name == "親"));
    ast.nodes[child as usize].metadata[0].owner_component_id = Some("foreign".into());
    assert!(
        collect(&ast)[0].children.is_none(),
        "forged owners cannot supply symbols"
    );
    ast.nodes[child as usize].metadata[0].owner_component_id =
        Some("fixture.semantic-provider".into());
    for bad in [
        json!({"version":2,"name":"子","kind":3,"selection":{"start":0,"end":3}}),
        json!({"version":1,"name":"子","kind":0,"selection":{"start":0,"end":3}}),
        json!({"version":1,"name":"子","kind":3,"selection":{"start":1,"end":3}}),
        json!({"version":1,"name":"子","kind":3,"selection":{"start":0,"end":999}}),
        json!({"version":1,"name":" ","kind":3,"selection":{"start":0,"end":3}}),
    ] {
        ast.nodes[child as usize].metadata[0].value = bad.to_string();
        assert!(collect(&ast)[0].children.is_none(), "bad contract {bad}");
    }
    ast.nodes[child as usize].metadata[0].value =
        json!({"version":1,"name":"子","kind":3,"selection":{"start":0,"end":3}}).to_string();
    let duplicate = ast.nodes[child as usize].metadata[0].clone();
    ast.nodes[child as usize].metadata.push(duplicate);
    assert!(
        collect(&ast)[0].children.is_none(),
        "duplicate contracts make no claim"
    );
    let mut document =
        Fixture::new("unused").finish_with(&[], Vec::new(), Vec::new(), "file:///outline.sk");
    document.symbols = symbols;
    let flat = document.document_symbols(false);
    assert_eq!(flat.as_array().unwrap().len(), 3);
    assert_eq!(flat[1]["containerName"], "親");
    assert_eq!(
        flat[1]["location"]["range"]["start"],
        json!({"line":2,"character":4})
    );
}

#[test]
fn document_symbols_reject_generated_source_and_ast_parentage_without_raw_ancestry() {
    let mut fixture = Fixture::new("one:\n    value\ntwo:\n    value");
    let a = fixture.push(SyntaxKind::Structure, "structure", fixture.line(0));
    let b = fixture.push(SyntaxKind::Section, "section", fixture.line(2));
    for id in [a, b] {
        fixture.nodes[id as usize].metadata.push(metadata(
            parser_wasm::DOCUMENT_SYMBOL_METADATA,
            json!({"version":1,"name":"outline","kind":3,"selection":{"start":0,"end":3}}),
        ));
    }
    fixture.nodes[a as usize].children.push(b);
    let mapped = skript_parser::MappedSource::identity(Arc::clone(&fixture.source));
    let raw = skript_parser::parse_raw_tree(
        &mapped,
        skript_parser::RawTreeOptions::for_skript_version(2, 15),
    );
    let ast = AstTree {
        roots: vec![a],
        nodes: fixture.nodes,
    };
    let symbols = document_symbols::from_ast(
        &ast,
        &raw,
        &mapped,
        &LineIndex::new(&fixture.source),
        &[],
        |_, _| true,
    );
    assert_eq!(symbols.len(), 1);
    assert!(
        symbols[0].children.is_none(),
        "AST parentage alone cannot make a sibling a child"
    );
    let other_source: Arc<str> = Arc::from(fixture.source.replace("value", "other"));
    let other = skript_parser::MappedSource::identity(Arc::clone(&other_source));
    assert!(document_symbols::from_ast(
        &ast, &raw, &other, &LineIndex::new(&other_source), &[], |_, _| true,
    ).is_empty(), "an unchanged header cannot lend an old body's original geometry");
    let mapped = skript_parser::MappedSource::new(
        Arc::clone(&fixture.source),
        Arc::clone(&fixture.source),
        Default::default(),
        vec![skript_parser::SourceMapSegment::new(
            TextRange::new(0, fixture.source.len()),
            skript_parser::SourceOrigin::replaced(TextRange::new(0, fixture.source.len()), None),
        )],
    )
    .unwrap();
    assert!(
        document_symbols::from_ast(
            &ast,
            &raw,
            &mapped,
            &LineIndex::new(&fixture.source),
            &[],
            |_, _| true
        )
        .is_empty()
    );
}

#[test]
fn document_symbols_cannot_borrow_an_enclosing_original_owner_extent() {
    let mut fixture = Fixture::new("outer:\n    inner:\n        value");
    let inner = fixture.push(SyntaxKind::Section, "section", fixture.line(1));
    fixture.nodes[inner as usize].metadata.push(metadata(
        parser_wasm::DOCUMENT_SYMBOL_METADATA,
        json!({"version":1,"name":"inner","kind":3,"selection":{"start":0,"end":5}}),
    ));
    let mapped = skript_parser::MappedSource::identity(Arc::clone(&fixture.source));
    let mut raw = skript_parser::parse_raw_tree(
        &mapped, skript_parser::RawTreeOptions::for_skript_version(2,15),
    );
    let owner = usize::try_from(raw.roots[0].get()).unwrap();
    let child = usize::try_from(raw.nodes[owner].children[0].get()).unwrap();
    raw.nodes[owner].code_span = raw.nodes[child].code_span.take();
    let ast = AstTree { roots: vec![inner], nodes: fixture.nodes };
    assert!(document_symbols::from_ast(
        &ast, &raw, &mapped, &LineIndex::new(&fixture.source), &[], |_, _| true,
    ).is_empty(), "an inner exact header is not the outer retained owner's header");
}

#[test]
#[ignore = "explicit semantic projection microbenchmark; no latency assertion"]
fn measure_semantic_projection_from_a_fixed_native_result() {
    fn observation(document: &Document) -> String {
        let mut declared = document.declared_parameters.iter().collect::<Vec<_>>();
        declared.sort_by_key(|(key, _)| *key);
        let mut unknown_scopes = document.unknown_variable_scopes.iter().collect::<Vec<_>>();
        unknown_scopes.sort_unstable();
        let value = json!({
            "occurrences": format!("{:?}", document.occurrences),
            "writes": format!("{:?}", document.writes),
            "functions": document.functions.iter().map(|symbol| format!("{:?}", (
                &symbol.definition.name, &symbol.definition.registration_id,
                &symbol.definition.parameters, &symbol.definition.return_type,
                symbol.definition.return_type_is_single, symbol.range,
                symbol.name_range, symbol.local))).collect::<Vec<_>>(),
            "calls": format!("{:?}", document.calls),
            "dependencies": format!("{:?}", document.dependencies),
            "declared": format!("{declared:?}"),
            "unknownScopes": unknown_scopes,
            "completionPoints": document.completion_points.iter().map(|point| json!({
                "range": [point.range.start, point.range.end],
                "ownerRange": [point.owner.range.start, point.owner.range.end],
                "owner": point.owner.identity, "parameter": point.owner.parameter,
                "requested": point.requested_types, "types": format!("{:?}", point.types)
            })).collect::<Vec<_>>(),
            "completionBindings": document.completion_type_bindings,
            "symbols": document.symbols,
            "renameComplete": document.rename_complete
        });
        format!("{:x}", Sha256::digest(serde_json::to_vec(&value).unwrap()))
    }
    let setup = std::time::Instant::now();
    let catalog = crate::lsp::test_support::runtime_catalog();
    let mut host = crate::new_parser_host(parser_wasm::HostConfig {
        syntax_catalog: Some(Arc::clone(&catalog)),
        runtime_profile: parser_wasm::RuntimeProfile {
            skript_version: Some("2.15.4".into()),
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap();
    let mut source = String::from(
        "# \u{65e5}\u{672c}\u{8a9e}\u{1f98a}\nfunction echo(value: text) :: text:\n    set {_copy} to {_value}\n    return {_copy}\nfunction wrapper(value: text) :: text:\n    return echo({_value})\n",
    );
    for i in 0..40 {
        source.push_str(&format!("on load:\n    set {{_value}} to \"text{i}\"\n    if {{_value}} is set:\n        send echo({{_value}}) to console\n"));
    }
    let source: Arc<str> = Arc::from(source);
    let uri: Uri = "file:///projection-benchmark/public.sk".parse().unwrap();
    let host_setup_ms = setup.elapsed().as_secs_f64() * 1000.0;
    let parse_started = std::time::Instant::now();
    let mut result = host
        .parse_document(
            parser_wasm::DocumentParseRequest::new(
                "file:///projection-benchmark",
                uri.as_str(),
                1,
                source.as_ref(),
            ),
            Default::default(),
        )
        .unwrap();
    assert_eq!(
        result.execution_status,
        parser_wasm::DocumentExecutionStatus::Complete
    );
    assert!(result.component_failures.is_empty());
    assert!(
        result.syntax.diagnostics.is_empty(),
        "{:?}",
        result.syntax.diagnostics
    );
    result.ast = result.editor_ast();
    let parse_ms = parse_started.elapsed().as_secs_f64() * 1000.0;
    let witnesses = result.semantic_witnesses();
    let owned_semantic_entries = result
        .ast
        .nodes
        .iter()
        .map(|node| {
            node.metadata
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
                    ) && entry
                        .owner_component_id
                        .as_deref()
                        .is_some_and(|issuer| witnesses.provider_valid(node, issuer))
                })
                .count()
        })
        .sum::<usize>();
    assert!(owned_semantic_entries > 0);
    // Compare the exact changed boundary with the previous algorithm outside
    // the timer, using witnesses for original nodes, never their clones.
    for original in &result.ast.nodes {
        let mut old = original.clone();
        let allowed = old
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
                ) && entry
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
        old.metadata.retain(|entry| {
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
        let mut new = original.clone();
        retain_semantic_contracts(&mut new, witnesses.unique_issuer(original));
        assert_eq!(format!("{:?}", new.metadata), format!("{:?}", old.metadata));
    }
    let first_started = std::time::Instant::now();
    let reference = Document::from_result(
        &result,
        Arc::clone(&source),
        uri.clone(),
        Arc::clone(&catalog),
    )
    .unwrap();
    let first_projection_ms = first_started.elapsed().as_secs_f64() * 1000.0;
    assert!(!reference.occurrences.is_empty());
    assert!(!reference.completion_points.is_empty());
    assert!(!reference.calls.is_empty());
    // Keep the original fingerprint comparable with the before measurement.
    // The extended fingerprint also covers function bodies and projection guards.
    let extended_observation = |document: &Document| {
        let value = format!(
            "{:?}",
            (
                &document.functions,
                &document.unavailable,
                &document.native_owner,
                document.function_policy,
                document.project_complete,
                &document.source_witness,
                &document.uri,
            )
        );
        format!("{:x}", Sha256::digest(value.as_bytes()))
    };
    let extended_expected = extended_observation(&reference);
    let expected = observation(&reference);
    let mut samples = Vec::new();
    for _ in 0..30 {
        let start = std::time::Instant::now();
        let document = Document::from_result(
            &result,
            Arc::clone(&source),
            uri.clone(),
            Arc::clone(&catalog),
        )
        .unwrap();
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(observation(&document), expected);
        assert_eq!(extended_observation(&document), extended_expected);
        std::hint::black_box(document);
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "projection_benchmark {}",
        json!({
            "sourceSha256": format!("{:x}", Sha256::digest(source.as_bytes())),
            "sourceBytes": source.len(), "astNodes": result.ast.nodes.len(),
            "hostSetupMs": host_setup_ms, "parseMs": parse_ms, "firstProjectionMs": first_projection_ms, "iterations": samples.len(),
            "ownedSemanticEntries": owned_semantic_entries,
            "projectionMedianMs": samples[15], "projectionP95Ms": samples[28],
            "observationSha256": expected,
            "extendedObservationSha256": extended_expected,
            "occurrences": reference.occurrences.len(), "calls": reference.calls.len(),
            "completionPoints": reference.completion_points.len()
        })
    );
}

#[test]
fn semantic_contract_filter_keeps_owned_duplicates_and_foreign_nonsemantic_entries() {
    for (issuer, kept) in [
        (Some("provider"), vec![0, 1, 2, 3, 6]),
        (Some("foreign"), vec![0, 4, 6]),
        (None, vec![0, 6]),
    ] {
        let entry = |key: &str, value: &str, owner: Option<&str>| MetadataEntry {
            key: key.into(),
            value: value.into(),
            owner_component_id: owner.map(str::to_owned),
        };
        let entries = vec![
            entry("ordinary", "no owner", None),
            entry(CHANGE, "same", Some("provider")),
            entry(CHANGE, "same", Some("provider")),
            entry(CHANGE, "conflict", Some("provider")),
            entry(FLOW, "branch", Some("foreign")),
            entry(PARAMETERS, "unowned", None),
            entry("skript.semantic.extension", "opaque", Some("foreign")),
        ];
        let value = |entry: &MetadataEntry| {
            (
                entry.key.clone(),
                entry.value.clone(),
                entry.owner_component_id.clone(),
            )
        };
        let expected = kept
            .into_iter()
            .map(|index| value(&entries[index]))
            .collect::<Vec<_>>();
        let mut ast = node("", 0, SyntaxKind::Effect, "effect", TextRange::new(0, 0));
        ast.metadata = entries;
        retain_semantic_contracts(&mut ast, issuer);
        assert_eq!(ast.metadata.iter().map(value).collect::<Vec<_>>(), expected);
        if issuer == Some("provider") {
            assert!(
                unique_metadata(&ast, CHANGE).is_none(),
                "duplicates stay ambiguous"
            );
        }
        for key in [
            CHANGE,
            FLOW,
            PARAMETERS,
            LOCAL_EFFECTS,
            OPERANDS,
            VALUE_RESULT,
            VARIABLE_READ,
            EVALUATION_CONTEXT,
        ] {
            ast.metadata = vec![
                entry(key, "contract", Some("provider")),
                entry(key, "unowned", None),
            ];
            retain_semantic_contracts(&mut ast, Some("provider"));
            assert_eq!(ast.metadata.len(), 1, "{key}");
            assert_eq!(ast.metadata[0].value, "contract");
            retain_semantic_contracts(&mut ast, None);
            assert!(ast.metadata.is_empty(), "{key}");
        }
    }
}

#[test]
fn accepted_assignment_and_branch_provenance_stays_current() {
    let mut fixture = Fixture::new(
        "owner:\n    bind {_a} = 1\n    branch:\n        bind {_a} = text\n    inspect {_a}",
    );
    let first = fixture.assign(1, "{_a}", "a", "1", "java.lang.Number", true, false, "set");
    let second = fixture.assign(
        3,
        "{_a}",
        "a",
        "text",
        "java.lang.String",
        true,
        false,
        "set",
    );
    let branch = fixture.container(2, SyntaxKind::Section, &[second], Some("branch"));
    let read = fixture.read(4, "{_a}", "a", true, false);
    let root = fixture.container(0, SyntaxKind::Structure, &[first, branch, read], None);
    let document = fixture.finish(&[root]);
    let (_, ty) = document.type_at(Position::new(4, 13)).unwrap();
    let InferredType::Values { support, .. } = ty else {
        panic!("{ty:?}")
    };
    assert!(support.is_proof());
    assert_eq!(
        support.evidence,
        BTreeSet::from([
            Evidence::AcceptedResult,
            Evidence::Assignment,
            Evidence::ControlFlowJoin
        ])
    );
    assert_eq!(document.definitions_at(Position::new(4, 13), &[]).len(), 2);
}

#[test]
fn type_inlays_use_post_write_roles_not_read_or_parameter_identity() {
    let mut fixture =
        Fixture::new("owner:\n    bind {_x} = 1\n    bind {_x} = text\n    inspect {_x}");
    let first = fixture.assign(1, "{_x}", "x", "1", "java.lang.Number", true, false, "set");
    let second = fixture.assign(
        2,
        "{_x}",
        "x",
        "text",
        "java.lang.String",
        true,
        false,
        "set",
    );
    let read = fixture.read(3, "{_x}", "x", true, false);
    let root = fixture.container(0, SyntaxKind::Structure, &[first, second, read], None);
    let document = fixture.finish(&[root]);
    let hints = document.write_type_hints();
    assert_eq!(hints.len(), 2);
    for (hint, line, label) in [(&hints[0], 1, ": number"), (&hints[1], 2, ": string")] {
        assert_eq!(hint.position, Position::new(line, 13));
        assert!(
            matches!(&hint.label, lsp_types::InlayHintLabel::String(actual) if actual == label)
        );
        assert_eq!(hint.kind, Some(lsp_types::InlayHintKind::TYPE));
        assert!(hint.text_edits.is_none());
    }
    assert_eq!(inferred(&document, 3, 13), string());
}

#[test]
fn invalidated_types_are_hover_history_only_and_fresh_writes_replace_them() {
    let mut fixture = Fixture::new(
        "owner:\n    bind {_x} = 1\n    opaque\n    inspect {_x}\n    bind {_x} = text\n    inspect {_x}",
    );
    let first = fixture.assign(1, "{_x}", "x", "1", "java.lang.Number", true, false, "set");
    let opaque = fixture.push(SyntaxKind::Effect, "effect", fixture.line(2));
    let old = fixture.read(3, "{_x}", "x", true, false);
    let fresh = fixture.assign(
        4,
        "{_x}",
        "x",
        "text",
        "java.lang.String",
        true,
        false,
        "set",
    );
    let read = fixture.read(5, "{_x}", "x", true, false);
    let root = fixture.container(
        0,
        SyntaxKind::Structure,
        &[first, opaque, old, fresh, read],
        None,
    );
    let document = fixture.finish(&[root]);
    assert_eq!(inferred(&document, 3, 13), InferredType::Unknown);
    let history = document.hover(Position::new(3, 13)).unwrap().1;
    assert!(history.contains("Current type: ?"), "{history}");
    assert!(
        history.contains("Previous type (history only): number"),
        "{history}"
    );
    assert!(
        history.contains("provider current-frame effects are unverified (line 3"),
        "{history}"
    );
    // History and unrelated later writes are not current definition evidence.
    assert!(
        matches!(&document.at(Position::new(3,13)).unwrap().reference,
        Reference::Variable { reaching, .. } if reaching.is_empty())
    );
    assert!(document.definitions_at(Position::new(3, 13), &[]).is_empty());
    assert_eq!(
        document.references_at(Position::new(3, 13), &[], true).len(),
        4
    );
    assert_eq!(inferred(&document, 5, 13), string());
    assert_eq!(
        document.definitions_at(Position::new(5, 13), &[])[0].range.start.line,
        4
    );
    assert!(
        !document
            .hover(Position::new(5, 13))
            .unwrap()
            .1
            .contains("history only")
    );
}

#[test]
fn opaque_operands_keep_history_without_current_types_or_reaching_proof() {
    for has_prior_invalidation in [false, true] {
        let source = if has_prior_invalidation {
            "owner:\n    bind {_x} = 1\n    opaque\n    inspect {_x}"
        } else {
            "owner:\n    bind {_x} = 1\n    inspect {_x}"
        };
        let mut fixture = Fixture::new(source);
        let write = fixture.assign(1, "{_x}", "x", "1", "java.lang.Number", true, false, "set");
        let opaque = has_prior_invalidation
            .then(|| fixture.push(SyntaxKind::Effect, "effect", fixture.line(2)));
        let read_row = if has_prior_invalidation { 3 } else { 2 };
        let read = fixture.read(read_row, "{_x}", "x", true, false);
        fixture.nodes[read as usize]
            .metadata
            .retain(|entry| entry.key != OPERANDS && entry.key != LOCAL_EFFECTS);
        let mut body = vec![write];
        body.extend(opaque);
        body.push(read);
        let root = fixture.container(0, SyntaxKind::Structure, &body, None);
        let document = fixture.finish(&[root]);
        let position = Position::new(read_row as u32, 13);
        assert_eq!(
            inferred(&document, position.line, position.character),
            InferredType::Unknown
        );
        let hover = document.hover(position).unwrap().1;
        assert!(hover.contains("Current type: ?"), "{hover}");
        assert!(
            hover.contains("Previous type (history only): number"),
            "{hover}"
        );
        assert!(
            hover.contains("provider current-frame effects are unverified (line 3"),
            "{hover}"
        );
        assert!(matches!(
            &document.at(position).unwrap().reference,
            Reference::Variable { reaching, .. } if reaching.is_empty()
        ));
        assert!(document.definitions_at(position, &[]).is_empty());
        assert!(
            document
                .completion_points
                .iter()
                .all(|point| point.types.is_empty())
        );
        assert_eq!(document.write_type_hints().len(), 1);
    }
}

#[test]
fn alias_history_never_becomes_a_current_heuristic_or_reaching_write() {
    let mut fixture =
        Fixture::new("owner:\n    bind {_a} = 1\n    bind {_A} = 2\n    inspect {_a}");
    let first = fixture.assign(1, "{_a}", "a", "1", "java.lang.Number", true, false, "set");
    let alias = fixture.assign(2, "{_A}", "A", "2", "java.lang.Number", true, false, "set");
    let read = fixture.read(3, "{_a}", "a", true, false);
    let root = fixture.container(0, SyntaxKind::Structure, &[first, alias, read], None);
    let document = fixture.finish(&[root]);
    let hover = document.hover(Position::new(3, 13)).unwrap().1;
    assert!(hover.contains("write may overlap"), "{hover}");
    assert_eq!(inferred(&document, 3, 13), InferredType::Unknown);
    // An uncertain alias cannot turn historical assignments into definitions.
    assert!(
        matches!(&document.at(Position::new(3,13)).unwrap().reference,
        Reference::Variable { reaching, .. } if reaching.is_empty())
    );
    assert!(document.definitions_at(Position::new(3, 13), &[]).is_empty());
}

#[test]
fn unverified_write_preserves_history_but_never_current_proof() {
    let mut fixture =
        Fixture::new("owner:\n    bind {_x} = 1\n    bind {_x} = text\n    inspect {_x}");
    let first = fixture.assign(1, "{_x}", "x", "1", "java.lang.Number", true, false, "set");
    let second = fixture.assign(
        2,
        "{_x}",
        "x",
        "text",
        "java.lang.String",
        true,
        false,
        "set",
    );
    fixture.nodes[second as usize]
        .metadata
        .retain(|entry| entry.key != CHANGE);
    fixture.nodes[second as usize].metadata.push(metadata(CHANGE,json!({"version":2,"mode":"set","targetCapture":0,"valueCapture":1,"identityDelta":false,"targetOnly":false})));
    let read = fixture.read(3, "{_x}", "x", true, false);
    let root = fixture.container(0, SyntaxKind::Structure, &[first, second, read], None);
    let document = fixture.finish(&[root]);
    assert_eq!(inferred(&document, 3, 13), InferredType::Unknown);
    let hover = document.hover(Position::new(3, 13)).unwrap().1;
    assert!(
        hover.contains("Previous type (history only): number"),
        "{hover}"
    );
    assert!(
        matches!(&document.write_type_hints()[1].label, lsp_types::InlayHintLabel::String(label) if label == ": ?")
    );
}

#[test]
fn history_only_branch_keys_do_not_create_new_current_candidates() {
    let mut fixture =
        Fixture::new("owner:\n    bind {_x} = 1\n    maybe:\n        opaque\n    inspect {_x}");
    let write = fixture.assign(1, "{_x}", "x", "1", "java.lang.Number", true, false, "set");
    let opaque = fixture.push(SyntaxKind::Effect, "effect", fixture.line(3));
    let branch = fixture.container(2, SyntaxKind::Section, &[opaque], Some("branch"));
    let read = fixture.read(4, "{_x}", "x", true, false);
    let root = fixture.container(0, SyntaxKind::Structure, &[write, branch, read], None);
    let document = fixture.finish(&[root]);
    assert_eq!(inferred(&document, 4, 13), InferredType::Unknown);
    // Static identity and references remain; old writes are not current definitions.
    assert!(
        matches!(&document.at(Position::new(4,13)).unwrap().reference,
        Reference::Variable { reaching, .. } if reaching.is_empty())
    );
    assert!(document.definitions_at(Position::new(4, 13), &[]).is_empty());
}

#[test]
fn binding_hint_anchor_is_utf16_and_rejected_geometry_has_no_hint() {
    let mut fixture = Fixture::new("owner:\n    😀 bind {_x} = 1\n    inspect {_x}");
    let write = fixture.assign(1, "{_x}", "x", "1", "java.lang.Number", true, false, "set");
    let read = fixture.read(2, "{_x}", "x", true, false);
    let root = fixture.container(0, SyntaxKind::Structure, &[write, read], None);
    let target = fixture.part(1, "{_x}");
    let document = fixture.finish(&[root]);
    let hint = &document.write_type_hints()[0];
    assert_eq!(
        hint.position.character,
        "    😀 bind {_x}".encode_utf16().count() as u32
    );
    assert_eq!(hint.position.line, 1);
    let mut blocked = Fixture::new("owner:\n    bind {_x} = 1");
    let write = blocked.assign(1, "{_x}", "x", "1", "java.lang.Number", true, false, "set");
    let root = blocked.container(0, SyntaxKind::Structure, &[write], None);
    let span = blocked.part(1, "{_x}");
    let document = blocked.finish_with(&[root], vec![span], Vec::new(), "file:///blocked.sk");
    assert!(document.write_type_hints().is_empty());
    assert!(target.end > target.start);
}

#[test]
fn duplicated_source_projections_cannot_mint_unique_original_write_locations() {
    let first=ExactProjection {virtual_range:TextRange::new(0,3),original:TextRange::new(0,3),context:0};
    let second=ExactProjection {virtual_range:TextRange::new(3,6),..first};
    assert!(!unique_projection(&projection_conflicts(&[first,second]),first.original));
    assert!(unique_projection(&projection_conflicts(&[first]),first.original));
    assert!(unique_projection(&projection_conflicts(&[second]),second.original),"unused original fragment is not a second execution");
    for other in [
        ExactProjection {virtual_range:TextRange::new(2,5),original:TextRange::new(1,4),..first},
        ExactProjection {virtual_range:TextRange::new(1,2),original:TextRange::new(0,1),..first},
        ExactProjection {context:1,..first},
    ] { assert!(!unique_projection(&projection_conflicts(&[first,other]),first.original)); }
    let child=ExactProjection {virtual_range:TextRange::new(1,2),original:TextRange::new(1,2),..first};
    let adjacent=ExactProjection {virtual_range:TextRange::new(7,9),original:TextRange::new(3,5),..first};
    assert!(projection_conflicts(&[first,child,adjacent]).is_empty());
    assert!(!unique_projection(&[],TextRange::empty(0)));
}

#[test]
fn reassigned_raw_body_membership_does_not_recover_reaching_definitions() {
    let mut fixture=Fixture::new("owner:\n    bind {_a} = 1\n    inspect {_a}\nother:\n    opaque\n");
    let write=fixture.assign(1,"{_a}","a","1","java.lang.Number",true,false,"set");
    let read=fixture.read(2,"{_a}","a",true,false);
    let root=fixture.container(0,SyntaxKind::Structure,&[write,read],None);
    let mapped=skript_parser::MappedSource::identity(Arc::clone(&fixture.source));
    let mut raw=skript_parser::parse_raw_tree(&mapped,skript_parser::RawTreeOptions::for_skript_version(2,15));
    let other=raw.roots[1];
    raw.nodes.iter_mut().find(|node|node.text.contains("bind {_a}")).unwrap().parent=Some(other);
    let document=Document::from_ast(fixture.source,"file:///fixture.sk".parse().unwrap(),catalog(),&AstTree {roots:vec![root],nodes:fixture.nodes},&raw,Vec::new(),Vec::new(),None,None,HashSet::new(),Vec::new(),None).unwrap();
    assert_eq!(inferred(&document,2,13),InferredType::Unknown);
    assert!(document.definitions_at(Position::new(2,13),&[]).is_empty());
}

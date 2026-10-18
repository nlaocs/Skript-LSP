//! Current-cursor grammar guidance. This never selects a semantic overload.
use super::*;
use lsp_types::{ParameterInformation, ParameterLabel, SignatureHelp, SignatureInformation};

const MAX_SIGNATURES: usize = 20;

#[derive(Debug, Clone, Copy, Default)]
pub(in crate::lsp) struct SignatureCapabilities {
    label_offsets: bool,
    per_signature_active: bool,
}
impl SignatureCapabilities {
    pub fn negotiate(client: Option<&lsp_types::SignatureHelpClientCapabilities>) -> Self {
        let info = client.and_then(|c| c.signature_information.as_ref());
        Self {
            label_offsets: info
                .and_then(|i| i.parameter_information.as_ref())
                .and_then(|p| p.label_offset_support)
                .unwrap_or(false),
            per_signature_active: info
                .and_then(|i| i.active_parameter_support)
                .unwrap_or(false),
        }
    }
    pub fn adapt(self, help: Option<SignatureHelp>) -> Option<SignatureHelp> {
        let mut help = help?;
        for (i, signature) in help.signatures.iter_mut().enumerate() {
            if !self.label_offsets {
                if let Some(parameters) = &mut signature.parameters {
                    let text = signature.label.encode_utf16().collect::<Vec<_>>();
                    let mut labels = HashSet::new();
                    let valid = parameters.iter_mut().all(|parameter| {
                        let ParameterLabel::LabelOffsets([start, end]) = parameter.label else {
                            return false;
                        };
                        let Some(units) = text.get(start as usize..end as usize) else {
                            return false;
                        };
                        let Ok(label) = String::from_utf16(units) else {
                            return false;
                        };
                        if !labels.insert(label.clone()) {
                            return false;
                        }
                        parameter.label = ParameterLabel::Simple(label);
                        true
                    });
                    if !valid {
                        signature.parameters = None;
                        signature.active_parameter = None;
                    }
                }
            }
            // Old clients use one help-level index. Other alternatives retain
            // their grammar but cannot accidentally inherit that parameter.
            if !self.per_signature_active && i > 0 {
                signature.parameters = None;
                signature.active_parameter = None;
            }
        }
        help.active_parameter = help.signatures.first().and_then(|s| s.active_parameter);
        if !self.per_signature_active {
            for signature in &mut help.signatures {
                signature.active_parameter = None;
            }
        }
        Some(help)
    }
}

pub(in crate::lsp) enum SignatureQuery {
    Ready(Option<SignatureHelp>),
    NeedsAnalysis,
}

impl Document {
    pub fn signature_at(
        &self,
        position: Position,
        references: bool,
    ) -> Result<SignatureQuery, String> {
        let offset = self
            .lines
            .offset(&self.source, position)
            .ok_or("invalid signature UTF-16 position")?;
        let Some(line) = self.context.get(&position.line) else {
            return Ok(SignatureQuery::Ready(None));
        };
        if line.blocked
            || offset < line.code_start
            || offset > line.range.end
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
            return Ok(SignatureQuery::Ready(None));
        }
        let query_line = self.structure_query_line(line, offset);
        let line = &query_line;
        if matches!(line.scope, Scope::Pending) {
            return Ok(SignatureQuery::NeedsAnalysis);
        }
        if matches!(line.scope, Scope::Raw | Scope::Entries(_)) {
            return Ok(SignatureQuery::Ready(None));
        }
        let prefix = &self.source[line.code_start..offset];
        if prefix.len() > MAX_CONTINUATION_BYTES {
            return Ok(SignatureQuery::Ready(None));
        }
        if matches!(lexical_context(prefix), Lexical::Comment) {
            return Ok(SignatureQuery::Ready(None));
        }
        let mut budget = continuation::Budget::new();
        if let Some(call) = call_cursor(prefix) {
            // Document functions can shadow Catalog functions, including void
            // signatures. A source-only view cannot answer this correctly.
            if !references {
                return Ok(SignatureQuery::NeedsAnalysis);
            }
            let mut signatures = Vec::new();
            let mut shadowed = HashSet::new();
            for function in &self.functions {
                if !budget.scan(function.name.len() + 1) {
                    return Ok(SignatureQuery::Ready(None));
                }
                if function.name == call.name {
                    if function.parameters.len() > MAX_TEMPLATE_BYTES {
                        return Ok(SignatureQuery::Ready(None));
                    }
                    if !budget.scan(function.parameters.iter().map(|p| p.1.len() + 1).sum()) {
                        return Ok(SignatureQuery::Ready(None));
                    }
                    shadowed.insert(function.shape());
                }
            }
            for (order, function) in self
                .functions
                .iter()
                .chain(&self.index.functions)
                .enumerate()
            {
                if !budget.scan(function.name.len() + 1) {
                    break;
                }
                if function.name != call.name {
                    continue;
                }
                if function.parameters.len() > MAX_TEMPLATE_BYTES {
                    continue;
                }
                if !budget.scan(
                    function
                        .parameters
                        .iter()
                        .map(|p| p.0.len() + p.1.len() + 4)
                        .sum(),
                ) {
                    break;
                }
                if order >= self.functions.len() && shadowed.contains(&function.shape()) {
                    continue;
                }
                if function.name.len()
                    + function
                        .parameters
                        .iter()
                        .map(|p| p.0.len() + p.1.len() + 7)
                        .sum::<usize>()
                    > MAX_TEMPLATE_BYTES
                {
                    continue;
                }
                let mut label = format!("{}(", function.name);
                let mut parameters = Vec::new();
                for (i, p) in function.parameters.iter().enumerate() {
                    if i > 0 {
                        label.push_str(", ");
                    }
                    let start = label.encode_utf16().count() as u32;
                    label.push_str(&format!(
                        "{}: {}{}{}",
                        p.0,
                        p.1,
                        if p.2 { "" } else { "[]" },
                        if p.3 { "?" } else { "" }
                    ));
                    parameters.push(parameter(start, label.encode_utf16().count() as u32));
                }
                label.push(')');
                // List parameters and named-argument permutations require
                // semantic binding. Keep the signature, omit an unproven index.
                let active = (!call.named
                    && call.argument < parameters.len()
                    && function
                        .parameters
                        .iter()
                        .take(call.argument + 1)
                        .all(|p| p.2))
                .then_some(call.argument as u32);
                signatures.push(SignatureInformation {
                    label,
                    documentation: Some(Documentation::String(function.detail.clone())),
                    parameters: active.map(|_| parameters),
                    active_parameter: active,
                });
                if signatures.len() == MAX_SIGNATURES {
                    break;
                }
            }
            if !signatures.is_empty() {
                return Ok(SignatureQuery::Ready(help(signatures)));
            }
        }
        if !matches!(lexical_context(prefix), Lexical::Code(_)) {
            return Ok(SignatureQuery::Ready(None));
        }
        let mut start = 0;
        let mut selected = Vec::new();
        for depth in 0..16 {
            let mut matches = Vec::new();
            for (order, pattern) in self.index.patterns.iter().enumerate() {
                if !budget.scan(1) {
                    break;
                }
                let allowed = if depth > 0 {
                    matches!(pattern.kind, SyntaxKind::Expression | SyntaxKind::Condition)
                } else if matches!(line.scope, Scope::Expressions) {
                    pattern.kind == SyntaxKind::Expression
                } else if matches!(line.scope, Scope::Conditions) {
                    pattern.kind == SyntaxKind::Condition
                } else if line.root {
                    matches!(pattern.kind, SyntaxKind::Event | SyntaxKind::Structure)
                } else {
                    matches!(
                        pattern.kind,
                        SyntaxKind::Effect | SyntaxKind::Section | SyntaxKind::Condition
                    )
                };
                if !allowed {
                    continue;
                }
                if budget.exhausted() {
                    break;
                }
                if let Some((fixed, slots, active_slot)) =
                    pattern.graph.signature(&prefix[start..], &mut budget)
                {
                    if !budget.scan(pattern.label.len() * (pattern.graph.parameters.len() + 2)) {
                        break;
                    }
                    let parameters = pattern
                        .graph
                        .parameters
                        .iter()
                        .map(|range| {
                            let start = pattern
                                .label
                                .get(..range.start)?
                                .encode_utf16()
                                .count()
                                .try_into()
                                .ok()?;
                            let end = pattern
                                .label
                                .get(..range.end)?
                                .encode_utf16()
                                .count()
                                .try_into()
                                .ok()?;
                            Some(parameter(start, end))
                        })
                        .collect::<Option<Vec<_>>>();
                    let active = active_slot.and_then(|(index, _)| {
                        parameters.as_ref()?.get(index).map(|_| index as u32)
                    });
                    matches.push((
                        fixed,
                        slots,
                        order,
                        SignatureInformation {
                            label: pattern.label.clone(),
                            documentation: Some(Documentation::String(pattern.detail.clone())),
                            // A missing/invalid index defaults to parameter zero in LSP.
                            // Omit the parameter list when evidence is ambiguous or the
                            // cursor is in fixed grammar, so clients cannot mislabel it.
                            parameters: active.and(parameters),
                            active_parameter: active,
                        },
                        active_slot.map(|(_, offset)| offset),
                    ));
                }
            }
            matches.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
            // Less-specific paths can swallow fixed words as an opaque argument.
            // Keep equally supported alternatives in Catalog order.
            let Some(best) = matches.first().map(|m| (m.0, m.1)) else {
                break;
            };
            matches.retain(|m| (m.0, m.1) == best);
            let next = matches
                .first()
                .and_then(|m| m.4)
                .filter(|offset| matches.iter().all(|m| m.4 == Some(*offset)));
            selected = matches
                .into_iter()
                .take(MAX_SIGNATURES)
                .map(|m| m.3)
                .collect();
            let Some(next) = next else {
                break;
            };
            let next = start + next;
            let next = next + prefix[next..].len() - prefix[next..].trim_start().len();
            if next <= start || next >= prefix.len() {
                break;
            }
            start = next;
        }
        Ok(SignatureQuery::Ready(help(selected)))
    }
}

fn parameter(start: u32, end: u32) -> ParameterInformation {
    ParameterInformation {
        label: ParameterLabel::LabelOffsets([start, end]),
        documentation: None,
    }
}
fn help(signatures: Vec<SignatureInformation>) -> Option<SignatureHelp> {
    (!signatures.is_empty()).then_some(SignatureHelp {
        signatures,
        active_signature: Some(0),
        active_parameter: None,
    })
}

struct CallCursor<'a> {
    name: &'a str,
    argument: usize,
    named: bool,
}
struct CallFrame<'a> {
    depth: usize,
    call: Option<CallCursor<'a>>,
    start: usize,
}
// Structural editing positions share quote/variable/interpolation frames with
// completion. Commas and parentheses follow native Function splitting rules.
fn call_cursor(text: &str) -> Option<CallCursor<'_>> {
    let mut frames = Vec::<CallFrame<'_>>::new();
    let mut cursor = cursor::LexicalCursor::new(text);
    for event in cursor.by_ref() {
        let offset = event.offset;
        if matches!(event.state, Lexical::Code(_)) {
            match event.character {
                '(' => {
                    let begin = text[..offset]
                        .char_indices()
                        .rev()
                        .find(|(_, c)| *c != '_' && !c.is_alphabetic() && !c.is_numeric())
                        .map_or(0, |(i, c)| i + c.len_utf8());
                    let name = &text[begin..offset];
                    let is_function = name.starts_with(|c: char| c == '_' || c.is_alphabetic());
                    frames.push(CallFrame {
                        depth: event.depth,
                        call: is_function.then_some(CallCursor {
                            name,
                            argument: 0,
                            named: false,
                        }),
                        start: offset + 1,
                    });
                }
                ')' => {
                    if frames.last()?.depth != event.depth {
                        return None;
                    }
                    frames.pop();
                }
                ',' | ':' => {
                    if let Some(frame) = frames.last_mut().filter(|f| f.depth == event.depth) {
                        if let Some(call) = &mut frame.call {
                            if event.character == ':' {
                                call.named = true;
                            } else if !text[frame.start..offset]
                                .trim_matches(|c| c <= '\u{20}')
                                .is_empty()
                            {
                                call.argument += 1;
                            }
                        }
                        if event.character == ',' {
                            frame.start = offset + 1;
                        }
                    }
                }
                '}' => return None,
                _ => {}
            }
        }
        if event.after_depth < event.depth {
            frames.retain(|frame| frame.depth <= event.after_depth);
        }
    }
    if matches!(cursor.state(), Lexical::Comment) {
        return None;
    }
    frames.into_iter().rev().find_map(|frame| frame.call)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn client_capabilities_negotiate_offsets_and_active_index_without_ambiguous_text_focus() {
        for offsets in [false, true] {
            for individual in [false, true] {
                let client = serde_json::from_value(serde_json::json!({"signatureInformation":{"activeParameterSupport":individual,"parameterInformation":{"labelOffsetSupport":offsets}}})).unwrap();
                let capabilities = SignatureCapabilities::negotiate(Some(&client));
                let raw = pattern_query("teleport %entities% to %location%", "teleport player to ")
                    .unwrap();
                let adapted = capabilities.adapt(Some(raw)).unwrap();
                let info = &adapted.signatures[0];
                assert_eq!(adapted.active_parameter, Some(1));
                assert_eq!(info.active_parameter, individual.then_some(1));
                let label = &info.parameters.as_ref().unwrap()[1].label;
                if offsets {
                    assert!(matches!(label, ParameterLabel::LabelOffsets(_)));
                } else {
                    assert_eq!(label, &ParameterLabel::Simple("%location%".into()));
                }
                let raw =
                    pattern_query("compare %objects% with %objects%", "compare 1 with ").unwrap();
                let repeated = capabilities.adapt(Some(raw)).unwrap();
                if !offsets {
                    assert!(repeated.signatures[0].parameters.is_none());
                    assert!(repeated.active_parameter.is_none());
                }
            }
        }
        assert!(!SignatureCapabilities::negotiate(None).label_offsets);
    }
    fn index(pattern: &str) -> Arc<CatalogIndex> {
        let rules = syntax_pattern_parser::syntax::PluralRules::from_json(include_str!(
            "../../../syntax-pattern-parser/tests/data/PluralRules-2.15.4.json"
        ))
        .unwrap();
        let parsed = syntax_pattern_parser::syntax::parse(pattern, &rules).unwrap();
        Arc::new(CatalogIndex {
            templates: Vec::new(),
            patterns: vec![Pattern {
                kind: SyntaxKind::Effect,
                label: pattern.into(),
                detail: "fixture".into(),
                identity: "fixture".into(),
                graph: continuation::Graph::new(&parsed.elements).unwrap(),
            }],
            functions: Vec::new(),
            limited: false,
            raw_options: skript_parser::RawTreeOptions::for_skript_version(2, 15),
        })
    }
    // Graph tests supply a confirmed executable scope; source-only views defer body classification.
    fn scoped_document(source: Arc<str>, index: Arc<CatalogIndex>) -> Result<Document, String> {
        let mut document = Document::from_source(source, index)?;
        for line in document.context.values_mut() {
            if !line.root {
                line.scope = Scope::Statements;
            }
        }
        Ok(document)
    }
    #[test]
    fn unknown_current_structure_scope_waits_before_giving_statement_guidance() {
        let document = Document::from_source(
            Arc::from("config:\n    teleport player to "),
            index("teleport %entities% to %location%"),
        )
        .unwrap();
        let position = document
            .lines
            .position(&document.source, document.source.len())
            .unwrap();
        assert!(matches!(
            document.signature_at(position, false).unwrap(),
            SignatureQuery::NeedsAnalysis
        ));
    }
    #[test]
    fn unknown_entry_key_cannot_be_lexically_classified_as_its_value() {
        // Entry keys may contain quotes/braces or exceed the value query budget.
        // Their boundary must be resolved before applying value lexical rules.
        for key in [
            "\"q".to_owned(),
            "{q".to_owned(),
            "q".repeat(MAX_CONTINUATION_BYTES + 1),
        ] {
            let document = Document::from_source(
                Arc::from(format!("config:\n    {key}: sin(")),
                index("teleport %entities% to %location%"),
            )
            .unwrap();
            let position = document
                .lines
                .position(&document.source, document.source.len())
                .unwrap();
            assert!(matches!(
                document.signature_at(position, false).unwrap(),
                SignatureQuery::NeedsAnalysis
            ));
        }
    }
    fn query(document: &Document, references: bool) -> Option<SignatureHelp> {
        let position = document
            .lines
            .position(&document.source, document.source.len())
            .unwrap();
        let SignatureQuery::Ready(help) = document.signature_at(position, references).unwrap()
        else {
            panic!("unexpected reference dependency");
        };
        help
    }
    fn pattern_query(pattern: &str, prefix: &str) -> Option<SignatureHelp> {
        let document =
            scoped_document(Arc::from(format!("on join:\n    {prefix}")), index(pattern)).unwrap();
        query(&document, false)
    }
    #[test]
    fn pattern_parameters_follow_structural_slots_and_preserve_original_utf16_labels() {
        for (pattern, prefix, expected) in [
            (
                "teleport %entities% to %location%",
                "teleport player",
                "%entities%",
            ),
            (
                "teleport %entities% to %location%",
                "teleport player to ",
                "%location%",
            ),
            (
                "[force] teleport %entities% (to|towards) %location%",
                "force teleport location(1, 2) towards ",
                "%location%",
            ),
            (
                "移動😀 %objects% to %location%",
                "移動😀 {_x} to ",
                "%location%",
            ),
            (
                "read <.+> as %objects%",
                "read custom value as ",
                "%objects%",
            ),
        ] {
            let help = pattern_query(pattern, prefix).unwrap();
            let info = &help.signatures[0];
            assert_eq!(info.label, pattern);
            let selected =
                &info.parameters.as_ref().unwrap()[info.active_parameter.unwrap() as usize];
            let ParameterLabel::LabelOffsets([start, end]) = selected.label else {
                panic!("UTF-16 offsets required");
            };
            let utf16 = info.label.encode_utf16().collect::<Vec<_>>();
            assert_eq!(
                String::from_utf16(&utf16[start as usize..end as usize]).unwrap(),
                expected,
                "{prefix}"
            );
        }
        let fixed =
            pattern_query("teleport %entities% to %location%", "teleport player t").unwrap();
        assert!(fixed.signatures[0].active_parameter.is_none());
        assert!(fixed.signatures[0].parameters.is_none());
        let ambiguous = pattern_query("move (%objects%|%locations%)", "move player").unwrap();
        assert!(ambiguous.signatures[0].active_parameter.is_none());
    }
    #[test]
    fn pattern_guidance_respects_source_context_and_budget() {
        for prefix in [
            "# teleport player to ",
            "send \"teleport player to ",
            "{teleport player to ",
            "unrelated ",
        ] {
            assert!(
                pattern_query("teleport %entities% to %location%", prefix).is_none(),
                "{prefix}"
            );
        }
        assert!(
            pattern_query(
                "teleport %entities% to %location%",
                &format!("teleport {}", "x".repeat(MAX_CONTINUATION_BYTES))
            )
            .is_none()
        );
        let root = scoped_document(
            Arc::from("teleport player to "),
            index("teleport %entities% to %location%"),
        )
        .unwrap();
        assert!(query(&root, false).is_none());
        assert!(root.signature_at(Position::new(0, 500), false).is_err());
    }
    #[test]
    fn adjacent_ast_literals_end_slots_and_unknown_call_spelling_keeps_registered_grammar() {
        for (pattern, source) in [
            (
                "[the] %number%(st|nd|rd|th) enchant[ment] offer",
                "the 1st enchantment ",
            ),
            ("%objects%'[s] name", "player's name"),
        ] {
            let help = pattern_query(pattern, source).unwrap();
            assert!(help.signatures[0].active_parameter.is_none(), "{source}");
        }
        let document = scoped_document(
            Arc::from("on join:\n    new SomeType(1, "),
            index(r"new %objects%\(%objects%\)"),
        )
        .unwrap();
        assert!(matches!(
            document
                .signature_at(
                    document
                        .lines
                        .position(&document.source, document.source.len())
                        .unwrap(),
                    false
                )
                .unwrap(),
            SignatureQuery::NeedsAnalysis
        ));
        let help = query(&document, true).unwrap();
        assert_eq!(help.signatures[0].active_parameter, Some(1));
        let mut comment = Document::test_fixture(Arc::from("choose(1 # comment"));
        comment.functions.push(Function {
            name: "choose".into(),
            identity: "fixture".into(),
            detail: "fixture".into(),
            returns_value: true,
            parameters: vec![("first".into(), "Number".into(), true, false)],
        });
        assert!(query(&comment, true).is_none());
    }
    #[test]
    fn shared_frames_keep_nested_strings_variable_quotes_and_raw_comment_endpoints_opaque() {
        let quoted =
            pattern_query("emit %objects% as %objects%", r#"emit "%f("a as b")%""#).unwrap();
        assert_eq!(quoted.signatures[0].active_parameter, Some(0));
        let variable =
            pattern_query("teleport %entities% to %location%", "teleport {_a\"b} to ").unwrap();
        assert_eq!(variable.signatures[0].active_parameter, Some(1));
        for source in ["choose(1 # comment", "send \"%choose(1 # comment"] {
            let mut document = scoped_document(
                Arc::from(format!("on join:\n    {source}")),
                index("send %objects%"),
            )
            .unwrap();
            document.functions.push(Function {
                name: "choose".into(),
                identity: "fixture".into(),
                detail: "fixture".into(),
                returns_value: true,
                parameters: vec![("first".into(), "Number".into(), true, false)],
            });
            assert!(query(&document, true).is_none(), "{source}");
        }
    }
    #[test]
    fn nested_expressions_and_leading_argument_conditions_share_the_graph() {
        for (outer, kind, inner, inner_kind, source, active) in [
            (
                "send %objects% [to %audiences%]",
                SyntaxKind::Effect,
                "distance between %locations% and %locations%",
                SyntaxKind::Expression,
                "send distance between player and ",
                1,
            ),
            (
                "if %conditions%",
                SyntaxKind::Section,
                "%objects% is between %objects% and %objects%",
                SyntaxKind::Condition,
                "if 1 is between 2 and ",
                2,
            ),
        ] {
            let mut patterns = index(outer);
            Arc::get_mut(&mut patterns).unwrap().patterns[0].kind = kind;
            let mut nested = Arc::try_unwrap(index(inner))
                .ok()
                .unwrap()
                .patterns
                .remove(0);
            nested.kind = inner_kind;
            Arc::get_mut(&mut patterns).unwrap().patterns.push(nested);
            let document =
                scoped_document(Arc::from(format!("on join:\n    {source}")), patterns).unwrap();
            let help = query(&document, false).unwrap();
            assert_eq!(help.signatures[0].label, inner);
            assert_eq!(help.signatures[0].active_parameter, Some(active));
        }
        let mut document = scoped_document(
            Arc::from("on join:\n    teleport player to "),
            index("teleport %entities% to %location%"),
        )
        .unwrap();
        document
            .blocked
            .push(TextRange::new(document.source.len(), document.source.len()));
        assert!(query(&document, false).is_none());
    }
    #[test]
    fn functions_wait_for_current_references_preserve_shadow_and_do_not_invent_binding() {
        let function = |class: &str, single| Function {
            name: "choose".into(),
            identity: class.into(),
            detail: class.into(),
            returns_value: true,
            parameters: vec![
                ("first".into(), class.into(), single, false),
                ("second".into(), class.into(), true, true),
            ],
        };
        let mut document = Document::test_fixture(Arc::from("choose(1, "));
        let index = Arc::get_mut(&mut document.index).unwrap();
        index.functions = vec![function("Number", true), function("Text", true)];
        document.functions = vec![Function {
            returns_value: false,
            detail: "local void".into(),
            ..function("Number", true)
        }];
        assert!(matches!(
            document.signature_at(Position::new(0, 10), false).unwrap(),
            SignatureQuery::NeedsAnalysis
        ));
        let help = query(&document, true).unwrap();
        assert_eq!(help.signatures.len(), 2);
        assert_eq!(
            help.signatures[0].documentation,
            Some(Documentation::String("local void".into()))
        );
        assert!(
            help.signatures
                .iter()
                .all(|s| s.active_parameter == Some(1))
        );
        for (source, single) in [
            ("choose(second: 1, ", true),
            ("choose(1, ", false),
            ("choose(1, 2, 3, ", true),
        ] {
            let mut document = Document::test_fixture(Arc::from(source));
            document.functions = vec![function("Number", single)];
            let info = query(&document, true).unwrap().signatures.remove(0);
            assert!(info.active_parameter.is_none(), "{source}");
            assert!(info.parameters.is_none());
        }
        let unknown = Document::test_fixture(Arc::from("unknown("));
        assert!(query(&unknown, true).is_none());
    }
    #[test]
    fn cursor_counts_only_commas_in_the_innermost_open_call() {
        for (text, name, argument, named) in [
            ("outer(", "outer", 0, false),
            ("outer(1, inner(2, ", "inner", 1, false),
            ("outer(1, inner(2, 3), ", "outer", 2, false),
            ("outer(\"a, b\", {_x, y}, (1, 2), ", "outer", 3, false),
            ("outer(\"a\"\", b\", ", "outer", 1, false),
            ("send \"%日本(1, ", "日本", 1, false),
            ("outer(second: 2, ", "outer", 1, true),
            ("outer(, , ", "outer", 0, false),
            ("outer({outer{inner},tail}, ", "outer", 1, false),
            ("outer([1,2], ", "outer", 2, false),
        ] {
            let call = call_cursor(text).unwrap();
            assert_eq!(
                (call.name, call.argument, call.named),
                (name, argument, named),
                "{text}"
            );
        }
        for text in [
            "outer(1)",
            "send \"outer(1,",
            "# outer(",
            "outer(1}",
            "{outer(1,",
            "send \"%日本😀(1, ",
        ] {
            assert!(call_cursor(text).is_none(), "{text}");
        }
    }
}

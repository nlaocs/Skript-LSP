use skript_parser::{
    ExpressionExpectedType, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionLeafParse,
    ExpressionLeafRequest, ExpressionLeafTiming, ExpressionNodeKind, ExpressionParseContext,
    ExpressionParseEnvironment, ExpressionParseRequest, ExpressionParserConfig, FunctionDefinition,
    FunctionLookupRequest, FunctionParameterDefinition, FunctionVersionPolicy, MappedSource,
    PatternHookControl, PatternHookEvent, PatternMatchEnvironment, TextRange,
    TypeExpressionOutcome, TypeExpressionRequest, parse_expression,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use syntaxes::{Catalog, ClassName, Multiplicity};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

#[derive(Default)]
struct DocumentFunctionEnvironment;

impl PatternMatchEnvironment for DocumentFunctionEnvironment {
    fn resolve_type(
        &mut self,
        _request: TypeExpressionRequest<'_>,
    ) -> Result<TypeExpressionOutcome, String> {
        Ok(TypeExpressionOutcome::default())
    }

    fn dispatch_hook(
        &mut self,
        _event: PatternHookEvent<'_>,
    ) -> Result<PatternHookControl, String> {
        Ok(PatternHookControl::Continue)
    }
}

impl ExpressionParseEnvironment for DocumentFunctionEnvironment {
    fn parse_expression_leaf(
        &mut self,
        request: ExpressionLeafRequest<'_>,
    ) -> Result<ExpressionLeafParse, String> {
        let candidate = request.candidate_ends.iter().rev().find_map(|end| {
            let range = TextRange::new(request.remaining.start, *end);
            let text = range.slice(request.input)?;
            text.parse::<i64>().ok().map(|_| ExpressionLeafCandidate {
                function: None,
                effects: None,
                parser_id: "test.number".to_owned(),
                kind: ExpressionLeafKind::Literal,
                timing: ExpressionLeafTiming::AfterRegistered,
                range,
                return_type: Some(ClassName("java.lang.Long".to_owned())),
                multiplicity: Some(Multiplicity::Single),
                children: Vec::new(),
                public_data: Vec::new(),
                metadata: BTreeMap::new(),
            })
        });
        Ok(candidate.into_iter().collect::<Vec<_>>().into())
    }

    fn lookup_functions(
        &mut self,
        request: FunctionLookupRequest<'_>,
    ) -> Result<Vec<FunctionDefinition>, String> {
        Ok(match request.name {
            "local" | "sin" => vec![document_function(request.name)],
            "named_disabled" => vec![named_arguments_disabled_function(request.name)],
            "wide_named" => vec![wide_named_function(request.name)],
            "plural_then_singles" => vec![three_number_function(request.name, false)],
            "single_then_singles" => vec![three_number_function(request.name, true)],
            _ => Vec::new(),
        })
    }

    fn state_revision(&self) -> Result<u64, String> {
        Ok(0)
    }
}

fn document_function(name: &str) -> FunctionDefinition {
    FunctionDefinition {
        parser_id: "document.function".to_owned(),
        name: name.to_owned(),
        definition_id: format!("document:function:{name}"),
        registration_id: format!("document:function:{name}:0"),
        registration_order: 0,
        return_type: Some(ClassName("java.lang.Number".to_owned())),
        return_type_is_single: true,
        parameters: vec![FunctionParameterDefinition {
            name: "value".to_owned(),
            parameter_type: ClassName("java.lang.Number".to_owned()),
            single: true,
            optional: false,
        }],
        metadata: BTreeMap::from([("function.source".to_owned(), "document".to_owned())]),
    }
}

fn three_number_function(name: &str, first_single: bool) -> FunctionDefinition {
    FunctionDefinition {
        parser_id: "document.function".to_owned(),
        name: name.to_owned(),
        definition_id: format!("document:function:{name}"),
        registration_id: format!("document:function:{name}:0"),
        registration_order: 0,
        return_type: Some(ClassName("java.lang.Number".to_owned())),
        return_type_is_single: true,
        parameters: vec![
            FunctionParameterDefinition {
                name: "values".to_owned(),
                parameter_type: ClassName("java.lang.Number".to_owned()),
                single: first_single,
                optional: false,
            },
            FunctionParameterDefinition {
                name: "second".to_owned(),
                parameter_type: ClassName("java.lang.Number".to_owned()),
                single: true,
                optional: false,
            },
            FunctionParameterDefinition {
                name: "third".to_owned(),
                parameter_type: ClassName("java.lang.Number".to_owned()),
                single: true,
                optional: false,
            },
        ],
        metadata: BTreeMap::new(),
    }
}

fn named_arguments_disabled_function(name: &str) -> FunctionDefinition {
    let mut definition = document_function(name);
    definition
        .metadata
        .insert("function.named-arguments".to_owned(), "false".to_owned());
    definition
}

fn wide_named_function(name: &str) -> FunctionDefinition {
    let mut definition = document_function(name);
    definition.parameters[0].name = "target value".to_owned();
    definition
}

#[test]
fn keeps_a_parenthesized_list_in_one_plural_parameter() {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let text = "plural_then_singles((1,2), 3, 4)";
    let source = MappedSource::identity(text);
    let node = parse_expression(
        snapshot.catalog(),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![ExpressionExpectedType {
                class_name: ClassName("java.lang.Object".to_owned()),
                plural: false,
            }],
            context: ExpressionParseContext::default(),
        },
        &mut DocumentFunctionEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("document Function parsing must succeed")
    .selected
    .expect("plural and trailing single parameters must parse")
    .node;

    let call = node.function.expect("Function identity must be retained");
    assert_eq!(call.arguments.len(), 3);
    assert_eq!(call.arguments[0].parameter_name, "values");
    assert_eq!(call.arguments[0].child_count, 2);
    assert_eq!(call.arguments[1].child_count, 1);
    assert_eq!(call.arguments[2].child_count, 1);
    assert_eq!(node.children.len(), 4);

    let ungrouped = "plural_then_singles(1, 2, 3, 4)";
    let source = MappedSource::identity(ungrouped);
    let result = parse_expression(
        snapshot.catalog(),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, ungrouped.len()),
            expected_types: vec![ExpressionExpectedType {
                class_name: ClassName("java.lang.Object".to_owned()),
                plural: false,
            }],
            context: ExpressionParseContext::default(),
        },
        &mut DocumentFunctionEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("wrong arity is a recoverable parse result");
    assert!(
        result.selected.is_none(),
        "outer commas must remain Function argument separators"
    );

    let singular = "single_then_singles((1,2), 3, 4)";
    let source = MappedSource::identity(singular);
    let result = parse_expression(
        snapshot.catalog(),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, singular.len()),
            expected_types: vec![ExpressionExpectedType {
                class_name: ClassName("java.lang.Object".to_owned()),
                plural: false,
            }],
            context: ExpressionParseContext::default(),
        },
        &mut DocumentFunctionEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("multiplicity mismatch is a recoverable parse result");
    assert!(
        result.selected.is_none(),
        "a single first parameter must reject a parenthesized list"
    );
}

#[test]
fn document_definitions_reuse_the_registered_function_call_ast() {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let catalog: &Catalog = snapshot.catalog();
    let text = "local(1)";
    let source = MappedSource::identity(text);
    let result = parse_expression(
        catalog,
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![ExpressionExpectedType {
                class_name: ClassName("java.lang.Object".to_owned()),
                plural: false,
            }],
            context: ExpressionParseContext::default(),
        },
        &mut DocumentFunctionEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("document Function lookup must use the native call parser");

    let selected = result.selected.expect("local Function must parse").node;
    assert!(matches!(
        selected.kind,
        ExpressionNodeKind::Function { ref parser_id } if parser_id == "document.function"
    ));
    let call = selected
        .function
        .expect("Function identity must be structured");
    assert_eq!(call.name, "local");
    assert_eq!(call.definition_id, "document:function:local");
    assert_eq!(call.arguments[0].parameter_name, "value");
    assert_eq!(selected.children.len(), 1);
}

#[test]
fn document_signature_shadows_the_same_catalog_signature() {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let text = "sin(1)";
    let source = MappedSource::identity(text);
    let result = parse_expression(
        snapshot.catalog(),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![ExpressionExpectedType {
                class_name: ClassName("java.lang.Object".to_owned()),
                plural: false,
            }],
            context: ExpressionParseContext::default(),
        },
        &mut DocumentFunctionEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("local namespace lookup must not conflict with the global signature");

    let call = result
        .selected
        .expect("document signature must win")
        .node
        .function
        .expect("Function must be structured");
    assert_eq!(call.definition_id, "document:function:sin");
}

#[test]
fn a_definition_policy_can_reject_named_call_arguments() {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let text = "named_disabled(value: 1)";
    let source = MappedSource::identity(text);
    let result = parse_expression(
        snapshot.catalog(),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![ExpressionExpectedType {
                class_name: ClassName("java.lang.Object".to_owned()),
                plural: false,
            }],
            context: ExpressionParseContext::default(),
        },
        &mut DocumentFunctionEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("a policy rejection must remain a recoverable parse result");

    assert!(
        result.selected.is_none(),
        "the call parser must honor a definition's named-argument policy"
    );
}

#[test]
fn named_argument_names_follow_the_skript_version_grammar() {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let text = "wide_named(target value: 1)";
    let source = MappedSource::identity(text);
    let parse = |policy| {
        let config = ExpressionParserConfig {
            function: policy,
            ..ExpressionParserConfig::default()
        };
        parse_expression(
            snapshot.catalog(),
            ExpressionParseRequest {
                source: &source,
                range: TextRange::new(0, text.len()),
                expected_types: vec![ExpressionExpectedType {
                    class_name: ClassName("java.lang.Object".to_owned()),
                    plural: false,
                }],
                context: ExpressionParseContext::default(),
            },
            &mut DocumentFunctionEnvironment,
            config,
        )
        .expect("Function parsing must remain recoverable")
    };

    assert!(
        parse(FunctionVersionPolicy::for_skript_version(2, 14, 3))
            .selected
            .is_none()
    );
    assert!(
        parse(FunctionVersionPolicy::for_skript_version(2, 15, 0))
            .selected
            .is_some()
    );
}

#[derive(Default)]
struct StatementEnvironment {
    visible: usize,
    applications: usize,
    semantic: Vec<(usize, usize)>,
    expressions: Vec<(usize, usize)>,
    effect_hooks: usize,
    cancel_after_argument: bool,
    cancelled: bool,
    cancel_empty_lookup: bool,
}

impl PatternMatchEnvironment for StatementEnvironment {
    fn cancellation_requested(&mut self) -> bool {
        self.cancelled
    }

    fn resolve_type(
        &mut self,
        _request: TypeExpressionRequest<'_>,
    ) -> Result<TypeExpressionOutcome, String> {
        Ok(TypeExpressionOutcome::default())
    }

    fn dispatch_hook(&mut self, event: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
        if matches!(
            event.kind,
            skript_parser::MatchSyntaxKind::Effect | skript_parser::MatchSyntaxKind::Section
        ) {
            self.effect_hooks += 1;
        }
        Ok(PatternHookControl::Continue)
    }
}

impl ExpressionParseEnvironment for StatementEnvironment {
    fn lookup_functions(
        &mut self,
        request: FunctionLookupRequest<'_>,
    ) -> Result<Vec<FunctionDefinition>, String> {
        if self.cancel_empty_lookup && request.name == "unknown_call" {
            self.cancelled = true;
        }
        match request.name {
            "procedure" => {
                let mut definition = document_function(request.name);
                definition.return_type = None;
                Ok(vec![definition])
            }
            "ambiguous" => {
                let first = document_function(request.name);
                let mut second = first.clone();
                second.registration_id.push_str(":long");
                second.parameters[0].parameter_type = ClassName("java.lang.Long".into());
                Ok(vec![first, second])
            }
            _ => DocumentFunctionEnvironment.lookup_functions(request),
        }
    }

    fn parse_expression_leaf(
        &mut self,
        request: ExpressionLeafRequest<'_>,
    ) -> Result<ExpressionLeafParse, String> {
        let mut result = DocumentFunctionEnvironment.parse_expression_leaf(request)?;
        for candidate in &mut result.candidates {
            candidate.effects = Some(skript_parser::ExpressionEffects::new((
                self.visible,
                self.visible + 1,
                1usize,
            )));
        }
        Ok(result)
    }

    fn begin_semantic_candidate(&mut self) -> Result<(), String> {
        self.semantic.push((self.visible, self.applications));
        Ok(())
    }

    fn finish_semantic_candidate(&mut self, accepted: bool) -> Result<(), String> {
        let before = self.semantic.pop().expect("balanced semantic scope");
        if !accepted {
            (self.visible, self.applications) = before;
        }
        Ok(())
    }

    fn begin_expression_candidate(&mut self) -> Result<(), String> {
        self.expressions.push((self.visible, self.applications));
        Ok(())
    }

    fn defer_expression_candidate(
        &mut self,
        accepted: bool,
    ) -> Result<Option<skript_parser::ExpressionEffects>, String> {
        let before = self.expressions.pop().expect("balanced expression scope");
        let after = self.visible;
        let applications = self.applications - before.1;
        (self.visible, self.applications) = before;
        Ok(
            accepted
                .then(|| skript_parser::ExpressionEffects::new((before.0, after, applications))),
        )
    }

    fn apply_expression_effects(
        &mut self,
        effects: &skript_parser::ExpressionEffects,
    ) -> Result<(), String> {
        let &(before, after, applications) = effects
            .downcast_ref::<(usize, usize, usize)>()
            .expect("this environment's effects");
        if self.visible == before {
            self.visible = after;
        } else {
            assert_eq!(
                self.visible, after,
                "deferred effects cannot overwrite unrelated state"
            );
        }
        // This count is restored/deferred with the transaction, but every
        // adoption attempt adds its complete selected argument application log.
        // Calling select_expression twice therefore cannot pass via idempotent value writes.
        self.applications += applications;
        if self.cancel_after_argument {
            self.cancelled = true;
        }
        Ok(())
    }

    fn state_revision(&self) -> Result<u64, String> {
        Ok(self.visible as u64)
    }
}

fn statement(
    text: &str,
    environment: &mut StatementEnvironment,
    policy: FunctionVersionPolicy,
) -> Result<skript_parser::EffectMatches, skript_parser::EffectParseError> {
    let snapshot = ssg::load(fixture()).unwrap();
    let source = MappedSource::identity(text);
    let tree = skript_parser::parse_raw_tree(
        &source,
        skript_parser::RawTreeOptions::for_skript_version(2, 15),
    );
    skript_parser::parse_effect(
        snapshot.catalog(),
        skript_parser::EffectParseRequest {
            source: &source,
            node: tree.get(tree.roots[0]).unwrap(),
            context: ExpressionParseContext::default(),
        },
        environment,
        skript_parser::EffectParserConfig {
            expression: ExpressionParserConfig {
                function: policy,
                ..ExpressionParserConfig::default()
            },
        },
    )
}

#[test]
fn function_statements_accept_void_and_discard_values_without_becoming_expressions() {
    for text in ["procedure(1)", "local(value: 1)", "sin(1)"] {
        let mut environment = StatementEnvironment::default();
        let parsed = statement(
            text,
            &mut environment,
            FunctionVersionPolicy::for_skript_version(2, 15, 4),
        )
        .unwrap();
        assert!(parsed.has_selection(), "{text}: {parsed:#?}");
        assert!(
            parsed.selected.is_none() && parsed.alternatives.is_empty() && parsed.unknown.is_none()
        );
        let node = parsed.selected_function().unwrap();
        assert_eq!(node.span.local_range.slice(text), Some(text));
        node.function
            .as_ref()
            .unwrap()
            .validate_children(node.children.len())
            .unwrap();
        assert_eq!(node.children.len(), 1);
        assert_eq!(
            environment.visible, 1,
            "only selected argument effects survive"
        );
        assert_eq!(
            environment.applications, 1,
            "selected arguments are adopted exactly once"
        );
        assert_eq!(
            environment.effect_hooks, 0,
            "native calls do not impersonate Effect registrations"
        );
        assert!(environment.semantic.is_empty() && environment.expressions.is_empty());
        if text.starts_with("procedure") {
            assert!(node.return_type.is_none() && node.multiplicity.is_none());
            let snapshot = ssg::load(fixture()).unwrap();
            let source = MappedSource::identity(text);
            let expression = parse_expression(
                snapshot.catalog(),
                ExpressionParseRequest {
                    source: &source,
                    range: TextRange::new(0, text.len()),
                    expected_types: vec![ExpressionExpectedType {
                        class_name: ClassName("java.lang.Object".into()),
                        plural: true,
                    }],
                    context: ExpressionParseContext::default(),
                },
                &mut StatementEnvironment::default(),
                ExpressionParserConfig::default(),
            )
            .unwrap();
            assert!(
                expression.selected.is_none(),
                "void remains invalid in a value position"
            );
        }
    }
}

#[test]
fn recognized_function_statement_errors_are_terminal_and_discard_argument_effects() {
    for text in [
        "unknown_call(1)",
        "local()",
        "local(1, 2)",
        "local(value: 1, value: 2)",
        "local(not_a_number)",
        "ambiguous(1)",
    ] {
        let mut environment = StatementEnvironment::default();
        let parsed = statement(
            text,
            &mut environment,
            FunctionVersionPolicy::for_skript_version(2, 15, 4),
        )
        .unwrap();
        assert!(parsed.function_rejected(), "{text}: {parsed:#?}");
        assert!(!parsed.has_selection());
        assert!(parsed.unknown.as_ref().unwrap().failures.fallback.is_some());
        assert_eq!(
            environment.visible, 0,
            "{text} leaked rejected candidate state"
        );
        assert_eq!(
            environment.applications, 0,
            "{text} retained an argument application"
        );
        assert_eq!(
            environment.effect_hooks, 0,
            "recognized invalid calls cannot fall through to registry"
        );
        assert!(environment.semantic.is_empty() && environment.expressions.is_empty());
    }
}

#[test]
fn function_statement_recognition_requires_complete_call_and_keeps_version_policy() {
    for text in [
        "local(1) trailing",
        "local(",
        "local (1)",
        "local(1) + local(2)",
        "send log(8) to console",
        "dummy effect registered through wrapper",
    ] {
        let parsed = statement(
            text,
            &mut StatementEnvironment::default(),
            FunctionVersionPolicy::for_skript_version(2, 15, 4),
        )
        .unwrap();
        assert!(
            matches!(
                parsed.native_function,
                skript_parser::NativeFunctionMatch::NotCall
            ),
            "{text}: {parsed:#?}"
        );
        if text.starts_with("local") {
            assert!(!parsed.has_selection(), "no prefix call acceptance: {text}");
        }
        if text == "dummy effect registered through wrapper" {
            assert!(
                parsed.selected.is_some(),
                "ordinary Effects still use the registry"
            );
        }
    }
    let old = statement(
        "wide_named(target value: 1)",
        &mut StatementEnvironment::default(),
        FunctionVersionPolicy::for_skript_version(2, 14, 3),
    )
    .unwrap();
    assert!(old.function_rejected());
    let current = statement(
        "wide_named(target value: 1)",
        &mut StatementEnvironment::default(),
        FunctionVersionPolicy::for_skript_version(2, 15, 0),
    )
    .unwrap();
    assert!(current.selected_function().is_some());
}

#[test]
fn function_statement_cancellation_unwinds_speculation_and_allows_the_next_parse() {
    let mut environment = StatementEnvironment {
        visible: 10,
        cancel_after_argument: true,
        ..Default::default()
    };
    let error = statement(
        "local(1)",
        &mut environment,
        FunctionVersionPolicy::for_skript_version(2, 15, 4),
    )
    .unwrap_err();
    assert!(
        skript_parser::parse_cancellation(&error).is_some(),
        "{error:?}"
    );
    assert_eq!(environment.visible, 10);
    assert_eq!(environment.applications, 0);
    assert!(environment.semantic.is_empty() && environment.expressions.is_empty());
    environment.cancel_after_argument = false;
    environment.cancelled = false;
    let parsed = statement(
        "local(1)",
        &mut environment,
        FunctionVersionPolicy::for_skript_version(2, 15, 4),
    )
    .unwrap();
    assert!(parsed.selected_function().is_some());
    assert_eq!(environment.visible, 11);
    assert_eq!(environment.applications, 1);
}

#[test]
fn function_statement_early_terminal_paths_preserve_typed_cancellation() {
    for environment in [
        StatementEnvironment {
            cancelled: true,
            ..Default::default()
        },
        StatementEnvironment {
            cancel_empty_lookup: true,
            ..Default::default()
        },
    ] {
        let mut environment = environment;
        let error = statement(
            "unknown_call(1)",
            &mut environment,
            FunctionVersionPolicy::for_skript_version(2, 15, 4),
        )
        .unwrap_err();
        assert!(
            skript_parser::parse_cancellation(&error).is_some(),
            "{error:?}"
        );
        assert_eq!((environment.visible, environment.applications), (0, 0));
        assert!(environment.semantic.is_empty() && environment.expressions.is_empty());
        assert_eq!(environment.effect_hooks, 0);
    }
}

#[derive(Default)]
struct NamespaceProbe {
    lookup_calls: usize,
    argument_leaf_calls: usize,
}

impl PatternMatchEnvironment for NamespaceProbe {
    fn resolve_type(
        &mut self,
        _request: TypeExpressionRequest<'_>,
    ) -> Result<TypeExpressionOutcome, String> {
        Ok(TypeExpressionOutcome::default())
    }

    fn dispatch_hook(
        &mut self,
        _event: PatternHookEvent<'_>,
    ) -> Result<PatternHookControl, String> {
        Ok(PatternHookControl::Continue)
    }
}

impl ExpressionParseEnvironment for NamespaceProbe {
    fn parse_expression_leaf(
        &mut self,
        request: ExpressionLeafRequest<'_>,
    ) -> Result<ExpressionLeafParse, String> {
        if request.remaining.slice(request.input) == Some("1") {
            self.argument_leaf_calls += 1;
        }
        DocumentFunctionEnvironment.parse_expression_leaf(request)
    }

    fn lookup_functions(
        &mut self,
        request: FunctionLookupRequest<'_>,
    ) -> Result<Vec<FunctionDefinition>, String> {
        self.lookup_calls += 1;
        DocumentFunctionEnvironment.lookup_functions(request)
    }

    fn state_revision(&self) -> Result<u64, String> {
        Ok(0)
    }
}

#[test]
fn incomplete_namespace_stops_document_and_catalog_calls_before_lookup_or_arguments() {
    let snapshot = ssg::load(fixture()).unwrap();
    assert!(!snapshot.catalog().functions_named("sin").is_empty());
    for text in ["local(1)", "sin(1)", "not_registered(1)"] {
        let source = MappedSource::identity(text);
        let mut environment = NamespaceProbe::default();
        let error = parse_expression(
            snapshot.catalog(),
            ExpressionParseRequest {
                source: &source,
                range: TextRange::new(0, text.len()),
                expected_types: vec![ExpressionExpectedType {
                    class_name: ClassName("java.lang.Object".into()),
                    plural: false,
                }],
                context: ExpressionParseContext::default(),
            },
            &mut environment,
            ExpressionParserConfig {
                function_namespace_complete: false,
                ..ExpressionParserConfig::default()
            },
        )
        .expect_err("missing namespace must not be interpreted as no-match or catalog success");
        let unavailable = skript_parser::function_namespace_unavailable(&error).unwrap();
        assert_eq!(
            unavailable.span.virtual_range,
            TextRange::new(0, text.len())
        );
        assert_eq!(unavailable.name, text.split('(').next().unwrap());
        assert_eq!(environment.lookup_calls, 0);
        assert_eq!(environment.argument_leaf_calls, 0);
        assert!(skript_parser::extension_fault(&error).is_none());
        assert!(skript_parser::parse_cancellation(&error).is_none());
    }
}

#[test]
fn incomplete_namespace_propagates_through_a_native_group_without_losing_call_range() {
    let snapshot = ssg::load(fixture()).unwrap();
    let text = "(local(1))";
    let source = MappedSource::identity(text);
    let error = parse_expression(
        snapshot.catalog(),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![ExpressionExpectedType {
                class_name: ClassName("java.lang.Object".into()),
                plural: false,
            }],
            context: ExpressionParseContext::default(),
        },
        &mut NamespaceProbe::default(),
        ExpressionParserConfig {
            function_namespace_complete: false,
            ..ExpressionParserConfig::default()
        },
    )
    .unwrap_err();
    let unavailable = skript_parser::function_namespace_unavailable(&error).unwrap();
    assert_eq!(unavailable.name, "local");
    assert_eq!(unavailable.span.virtual_range.slice(text), Some("local(1)"));
}

#[test]
fn complete_namespace_default_still_selects_document_function_and_parses_its_argument() {
    let snapshot = ssg::load(fixture()).unwrap();
    let text = "local(1)";
    let source = MappedSource::identity(text);
    let mut environment = NamespaceProbe::default();
    let result = parse_expression(
        snapshot.catalog(),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![ExpressionExpectedType {
                class_name: ClassName("java.lang.Object".into()),
                plural: false,
            }],
            context: ExpressionParseContext::default(),
        },
        &mut environment,
        ExpressionParserConfig::default(),
    )
    .unwrap();
    assert_eq!(
        result.selected.unwrap().node.function.unwrap().name,
        "local"
    );
    assert!(environment.lookup_calls > 0);
    assert!(environment.argument_leaf_calls > 0);
}

use skript_parser::{
    ExpressionLeafCandidate, ExpressionLeafKind, ExpressionLeafParse, ExpressionLeafRequest,
    ExpressionParseContext, ExpressionParseEnvironment, MappedSource, PatternHookControl,
    PatternHookEvent, PatternMatchEnvironment, RawTreeOptions, SectionBodyMode, SectionBodyNode,
    SectionChildrenDecision, SectionChildrenRequest, SectionExitDecision, SectionParseRequest,
    SectionParserConfig, TextRange, TypeExpressionOutcome, TypeExpressionRequest, parse_raw_tree,
    parse_section,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use syntaxes::{
    Catalog, CatalogParts, ClassName, DefinitionId, Multiplicity, Pattern, RegistrationId, Syntax,
};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

#[derive(Default)]
struct ScopedEnvironment {
    literal_depths: BTreeMap<String, String>,
    exit_depths: Vec<String>,
    condition_body: bool,
    reject_exit: bool,
    reject_exit_registration: Option<String>,
}

impl PatternMatchEnvironment for ScopedEnvironment {
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

impl ExpressionParseEnvironment for ScopedEnvironment {
    fn parse_expression_leaf(
        &mut self,
        request: ExpressionLeafRequest<'_>,
    ) -> Result<ExpressionLeafParse, String> {
        let Some((range, value)) = request.candidate_ends.iter().rev().find_map(|end| {
            let range = TextRange::new(request.remaining.start, *end);
            let text = range.slice(request.input)?;
            (text.starts_with('"') && text.ends_with('"')).then(|| (range, text.to_owned()))
        }) else {
            return Ok(ExpressionLeafParse::default());
        };
        self.literal_depths.insert(
            value,
            request
                .context
                .values
                .get("scope-depth")
                .cloned()
                .unwrap_or_else(|| "0".to_owned()),
        );
        Ok(vec![ExpressionLeafCandidate {
            parser_id: "test.string".to_owned(),
            kind: ExpressionLeafKind::Literal,
            range,
            return_type: Some(ClassName("java.lang.String".to_owned())),
            multiplicity: Some(Multiplicity::Single),
            children: Vec::new(),
            metadata: BTreeMap::new(),
        }]
        .into())
    }

    fn enter_section_children(
        &mut self,
        request: SectionChildrenRequest<'_>,
    ) -> Result<SectionChildrenDecision, String> {
        let mut context = request.context.clone();
        let depth = context
            .values
            .get("scope-depth")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0)
            + 1;
        context
            .values
            .insert("scope-depth".to_owned(), depth.to_string());
        let mut metadata = request.metadata.clone();
        metadata.insert("test.enter-depth".to_owned(), depth.to_string());
        Ok(SectionChildrenDecision::Accept {
            context,
            body_mode: if self.condition_body {
                SectionBodyMode::Conditions
            } else {
                request.body_mode
            },
            metadata,
        })
    }

    fn exit_section_children(
        &mut self,
        request: SectionChildrenRequest<'_>,
    ) -> Result<SectionExitDecision, String> {
        if self.reject_exit
            || self.reject_exit_registration.as_deref() == Some(request.registration_id)
        {
            return Ok(SectionExitDecision::Reject {
                reason: "rejected after parsing the body".to_owned(),
                diagnostics: Vec::new(),
            });
        }
        self.exit_depths.push(
            request
                .context
                .values
                .get("scope-depth")
                .cloned()
                .unwrap_or_else(|| "0".to_owned()),
        );
        let mut context = request.context.clone();
        let depth = context
            .values
            .get("scope-depth")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0)
            .saturating_sub(1);
        context
            .values
            .insert("scope-depth".to_owned(), depth.to_string());
        Ok(SectionExitDecision::Accept {
            context,
            metadata: request.metadata.clone(),
        })
    }

    fn state_revision(&self) -> Result<u64, String> {
        Ok(0)
    }
}

fn alternative_section_catalog() -> Catalog {
    let snapshot = ssg::load(fixture()).expect("schema 4 fixture must load");
    let source = snapshot.catalog();
    let source_view = source.source().cloned().expect("SSG source view");
    let template = source
        .sections()
        .find(|section| {
            section.common.element_class.as_str()
                == "jp.nlaocs.skriptDummyAddon.fixture.LegacySyntaxes$DummySection"
        })
        .expect("fixture Section");
    let pattern_source = "lifecycle section";
    let parsed = syntax_pattern_parser::syntax::parse(pattern_source, source.plural_rules())
        .expect("test Section pattern must parse");

    let mut first = template.clone();
    first.common.definition_id = DefinitionId("section:test:first".to_owned());
    first.common.registration_id = RegistrationId("section:test:first:0".to_owned());
    first.common.registration_order = usize::MAX - 1;
    first.common.element_class = ClassName("test.FirstSection".to_owned());
    first.common.patterns = vec![Pattern {
        source: pattern_source.to_owned(),
        parsed: parsed.clone(),
    }];

    let mut fallback = first.clone();
    fallback.common.definition_id = DefinitionId("section:test:fallback".to_owned());
    fallback.common.registration_id = RegistrationId("section:test:fallback:0".to_owned());
    fallback.common.registration_order = usize::MAX;
    fallback.common.element_class = ClassName("test.FallbackSection".to_owned());

    let mut syntaxes = source
        .syntaxes()
        .iter()
        .filter(|syntax| {
            !matches!(
                syntax,
                Syntax::Section(section)
                    if section.common.element_class.as_str()
                        == "jp.nlaocs.skriptDummyAddon.fixture.LegacySyntaxes$DummySection"
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    syntaxes.extend([Syntax::Section(first), Syntax::Section(fallback)]);
    Catalog::new(CatalogParts {
        syntaxes,
        converters: source.converters().to_vec(),
        comparators: source.comparators().to_vec(),
        event_values: source.event_values().to_vec(),
        properties: source.properties().to_vec(),
        operators: source.operators().to_vec(),
        operations: source.operations().clone(),
        differences: source.differences().to_vec(),
        classes: source.classes().to_vec(),
        aliases: source.aliases().clone(),
        plural_rules: source.plural_rules().clone(),
        language: source
            .language_entries()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect(),
    })
    .with_unchecked_source(source_view)
}

#[test]
fn section_exit_rejection_is_a_recoverable_candidate_failure() {
    let snapshot = ssg::load(fixture()).expect("schema 4 fixture must load");
    let input = "dummy fixture section:\n    dummy fixture condition\n";
    let source = MappedSource::identity(input);
    let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
    let node = tree.get(tree.roots[0]).expect("fixture Section root");
    let mut environment = ScopedEnvironment {
        reject_exit: true,
        ..ScopedEnvironment::default()
    };
    let result = parse_section(
        snapshot.catalog(),
        SectionParseRequest {
            source: &source,
            tree: &tree,
            node,
            context: ExpressionParseContext::default(),
        },
        &mut environment,
        SectionParserConfig::default(),
    )
    .expect("exit rejection must not abort the parser");

    assert!(result.selected.is_none());
    let trace = result
        .unknown
        .expect("rejected Section remains source preserving")
        .failure
        .expect("exit rejection is ranked");
    assert!(matches!(
        trace.root_cause().failure.reasons.as_slice(),
        [skript_parser::PatternFailureReason::HookRejected { reason }]
            if reason == "rejected after parsing the body"
    ));
    assert!(
        trace
            .frame
            .expect("semantic rejection keeps syntax identity")
            .registration_id
            .starts_with("section:skriptdummyaddon:")
    );
}

#[test]
fn section_exit_rejection_retries_a_later_candidate() {
    let catalog = alternative_section_catalog();
    let input = "lifecycle section:\n";
    let source = MappedSource::identity(input);
    let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
    let node = tree.get(tree.roots[0]).expect("fixture Section root");
    let mut environment = ScopedEnvironment {
        reject_exit_registration: Some("section:test:first:0".to_owned()),
        ..ScopedEnvironment::default()
    };
    let result = parse_section(
        &catalog,
        SectionParseRequest {
            source: &source,
            tree: &tree,
            node,
            context: ExpressionParseContext::default(),
        },
        &mut environment,
        SectionParserConfig::default(),
    )
    .expect("a rejected Section candidate must remain recoverable");

    let selected = result
        .selected
        .as_ref()
        .expect("the fallback Section must be selected");
    assert_eq!(selected.matched.registration_id, "section:test:fallback:0");
    assert!(result.unknown.is_none());
}

#[test]
fn addon_can_select_a_condition_only_section_body() {
    let snapshot = ssg::load(fixture()).unwrap();
    let input =
        "dummy fixture section:\n    dummy fixture condition\n    dummy fixture condition\n";
    let source = MappedSource::identity(input);
    let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
    let node = tree.get(tree.roots[0]).unwrap();
    let mut environment = ScopedEnvironment {
        condition_body: true,
        ..ScopedEnvironment::default()
    };
    let result = parse_section(
        snapshot.catalog(),
        SectionParseRequest {
            source: &source,
            tree: &tree,
            node,
            context: ExpressionParseContext::default(),
        },
        &mut environment,
        SectionParserConfig::default(),
    )
    .expect("condition-only Section body must parse");

    let selected = result.selected.expect("fixture Section must match");
    assert_eq!(selected.body_mode, SectionBodyMode::Conditions);
    assert_eq!(selected.body.len(), 2);
    assert!(selected.body.iter().all(|node| matches!(
        node,
        SectionBodyNode::Condition { matches, .. } if matches.selected.is_some()
    )));
}

#[test]
fn nested_section_context_is_pushed_for_children_and_popped_for_siblings() {
    let snapshot = ssg::load(fixture()).unwrap();
    let input = "dummy fixture section:\n    dummy fixture section:\n        run dummy fixture effect with \"nested\"\n    run dummy fixture effect with \"outer\"\n";
    let source = MappedSource::identity(input);
    let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
    let node = tree.get(tree.roots[0]).unwrap();
    let mut environment = ScopedEnvironment::default();
    let result = parse_section(
        snapshot.catalog(),
        SectionParseRequest {
            source: &source,
            tree: &tree,
            node,
            context: ExpressionParseContext::default(),
        },
        &mut environment,
        SectionParserConfig::default(),
    )
    .expect("nested fixture Sections must parse");

    let selected = result.selected.as_ref().expect("root Section must match");
    assert_eq!(
        selected
            .metadata
            .get("test.enter-depth")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        environment
            .literal_depths
            .get("\"nested\"")
            .map(String::as_str),
        Some("2")
    );
    assert_eq!(
        environment
            .literal_depths
            .get("\"outer\"")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(environment.exit_depths, ["2", "1"]);
}

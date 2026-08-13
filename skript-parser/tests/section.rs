use skript_parser::{
    ExpressionLeafCandidate, ExpressionLeafKind, ExpressionLeafRequest, ExpressionParseContext,
    ExpressionParseEnvironment, MappedSource, PatternHookControl, PatternHookEvent,
    PatternMatchEnvironment, RawTreeOptions, SectionChildrenDecision, SectionChildrenRequest,
    SectionParseRequest, SectionParserConfig, TextRange, TypeExpressionOutcome,
    TypeExpressionRequest, parse_raw_tree, parse_section,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use syntaxes::{ClassName, Multiplicity};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

#[derive(Default)]
struct ScopedEnvironment {
    literal_depths: BTreeMap<String, String>,
    exit_depths: Vec<String>,
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
    ) -> Result<Vec<ExpressionLeafCandidate>, String> {
        let Some((range, value)) = request.candidate_ends.iter().rev().find_map(|end| {
            let range = TextRange::new(request.remaining.start, *end);
            let text = range.slice(request.input)?;
            (text.starts_with('"') && text.ends_with('"')).then(|| (range, text.to_owned()))
        }) else {
            return Ok(Vec::new());
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
            metadata: BTreeMap::new(),
        }])
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
        Ok(SectionChildrenDecision::Accept(context))
    }

    fn exit_section_children(&mut self, request: SectionChildrenRequest<'_>) -> Result<(), String> {
        self.exit_depths.push(
            request
                .context
                .values
                .get("scope-depth")
                .cloned()
                .unwrap_or_else(|| "0".to_owned()),
        );
        Ok(())
    }

    fn state_revision(&self) -> Result<u64, String> {
        Ok(0)
    }
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

    assert!(result.selected.is_some());
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

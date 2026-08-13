use skript_parser::{
    ExpressionExpectedType, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionLeafRequest,
    ExpressionNodeKind, ExpressionParseContext, ExpressionParseEnvironment, ExpressionParseRequest,
    ExpressionParserConfig, FunctionDefinition, FunctionLookupRequest, FunctionParameterDefinition,
    MappedSource, PatternHookControl, PatternHookEvent, PatternMatchEnvironment, TextRange,
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
    ) -> Result<Vec<ExpressionLeafCandidate>, String> {
        let candidate = request.candidate_ends.iter().rev().find_map(|end| {
            let range = TextRange::new(request.remaining.start, *end);
            let text = range.slice(request.input)?;
            text.parse::<i64>().ok().map(|_| ExpressionLeafCandidate {
                parser_id: "test.number".to_owned(),
                kind: ExpressionLeafKind::Literal,
                range,
                return_type: Some(ClassName("java.lang.Long".to_owned())),
                multiplicity: Some(Multiplicity::Single),
                metadata: BTreeMap::new(),
            })
        });
        Ok(candidate.into_iter().collect())
    }

    fn lookup_functions(
        &mut self,
        request: FunctionLookupRequest<'_>,
    ) -> Result<Vec<FunctionDefinition>, String> {
        Ok(match request.name {
            "local" | "sin" => vec![document_function(request.name)],
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

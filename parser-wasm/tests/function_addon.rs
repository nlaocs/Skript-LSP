use parser_wasm::{
    host::{HostConfig, InvocationContext, ParserHost},
    state::{NamespaceVisibility, StateScope},
};
use skript_parser::{
    ExpressionExpectedType, ExpressionParseContext, ExpressionParseRequest, ExpressionParserConfig,
    MappedSource, TextRange,
};
use std::{path::Path, sync::Arc};
use syntaxes::{Catalog, ClassName};

fn function_host(max_generated_output_bytes: usize) -> ParserHost {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let snapshot = ssg::load_for_parsing(
        root.join("utilities/skript-repl/tests/data/current-multi-addon-2.15.4"),
    )
    .unwrap();
    let catalog = Arc::<Catalog>::new(snapshot.catalog().clone());
    let core = std::fs::read(root.join("artifacts/core-library.wasm")).unwrap();
    let addon = std::fs::read(root.join("artifacts/function-addon.wasm")).unwrap();
    let mut host = ParserHost::new(
        &core,
        HostConfig {
            syntax_catalog: Some(catalog),
            max_generated_output_bytes,
            ..HostConfig::default()
        },
    )
    .unwrap();
    host.load_addon(&addon).unwrap();
    host
}

#[test]
#[ignore = "requires preserved WIT 0.39 / ABI 21 CoreLibrary and Function addon"]
fn diagnoses_previous_wit_components() {
    let old_core = std::fs::read(
        std::env::var_os("SKRIPT_PREVIOUS_CORE_LIBRARY").expect("set SKRIPT_PREVIOUS_CORE_LIBRARY"),
    )
    .unwrap();
    let old_addon = std::fs::read(
        std::env::var_os("SKRIPT_PREVIOUS_FUNCTION_COMPONENT")
            .expect("set SKRIPT_PREVIOUS_FUNCTION_COMPONENT"),
    )
    .unwrap();
    let mut config = HostConfig::default();
    config.runtime_profile.skript_version = Some("2.15.4".into());
    let error = match ParserHost::new(&old_core, config) {
        Err(error) => error,
        Ok(_) => panic!("previous CoreLibrary must be rejected"),
    };
    eprintln!("previous-core: {error}");
    assert!(error.to_string().contains("@0.39.0"));
    let mut host = function_host(HostConfig::default().max_generated_output_bytes);
    let error = host.load_addon(&old_addon).unwrap_err();
    eprintln!("previous-addon: {error}");
    assert!(error.to_string().contains("@0.39.0"));
    let transaction = host
        .begin_parse("file:///workspace", "file:///function.sk", 1)
        .unwrap();
    let source = MappedSource::identity("sin(abs(-1))");
    let parsed = host
        .parse_expression_in_parse(
            &transaction,
            InvocationContext {
                invocation_id: 1,
                subscription_id: String::new(),
                document_id: "file:///function.sk".into(),
                document_revision: 1,
                expansion: None,
                syntax_context: 0,
            },
            ExpressionParseRequest {
                source: &source,
                range: TextRange::new(0, source.virtual_source().len()),
                expected_types: Vec::new(),
                context: ExpressionParseContext::default(),
            },
            ExpressionParserConfig::default(),
        )
        .unwrap();
    assert!(parsed.matches.selected.is_some());
    transaction.cancel().unwrap();
}

#[test]
fn invalid_function_output_rolls_back_its_state_and_keeps_the_host_usable() {
    let mut host = function_host(HostConfig::default().max_generated_output_bytes);
    for (index, mode) in [
        "range",
        "overlap",
        "unowned",
        "omitted",
        "name",
        "missing",
        "kind",
        "span",
        "reference",
    ]
    .iter()
    .enumerate()
    {
        let revision = index as u64 + 1;
        let transaction = host
            .begin_parse(
                "file:///workspace",
                "file:///workspace/function.sk",
                revision,
            )
            .unwrap();
        let text = format!("wasm bad-{mode}(wasm echo(\"x\"))");
        let source = MappedSource::identity(text.as_str());
        let error = host
            .parse_expression_in_parse(
                &transaction,
                InvocationContext {
                    invocation_id: revision,
                    subscription_id: String::new(),
                    document_id: "file:///workspace/function.sk".into(),
                    document_revision: revision,
                    expansion: None,
                    syntax_context: 0,
                },
                ExpressionParseRequest {
                    source: &source,
                    range: TextRange::new(0, text.len()),
                    expected_types: vec![ExpressionExpectedType {
                        class_name: ClassName("java.lang.Object".into()),
                        plural: true,
                    }],
                    context: ExpressionParseContext::default(),
                },
                ExpressionParserConfig::default(),
            )
            .expect_err("malformed Function data must fault");
        assert_eq!(
            skript_parser::extension_fault(&error).unwrap().code,
            "parser.extension-invalid-output",
            "{mode}"
        );
        assert!(
            transaction.read_write_set().unwrap().writes.is_empty(),
            "{mode}"
        );
        let mut invocation = transaction.begin_invocation("test.function-addon").unwrap();
        assert!(
            invocation
                .get(
                    StateScope::Parse,
                    NamespaceVisibility::Private,
                    "function-state",
                    "invalid-function"
                )
                .unwrap()
                .is_none(),
            "{mode}"
        );
        invocation.rollback();
        transaction.cancel().unwrap();
    }
}

#[test]
fn function_metadata_alone_exceeding_output_quota_rolls_back_nested_state() {
    const LIMIT: usize = 1024 * 1024;
    let mut host = function_host(LIMIT);
    for (index, field) in [
        "name",
        "definition",
        "registration",
        "parameter",
        "supplied",
    ]
    .iter()
    .enumerate()
    {
        let revision = index as u64 + 1;
        for oversized in [false, true, false] {
            let transaction = host
                .begin_parse(
                    "file:///workspace",
                    "file:///workspace/function.sk",
                    revision,
                )
                .unwrap();
            let text = if oversized {
                format!("wasm quota-{field}(wasm echo(\"x\"))")
            } else {
                "wasm echo(wasm echo(\"x\"))".to_owned()
            };
            let source = MappedSource::identity(text.as_str());
            let result = host.parse_expression_in_parse(
                &transaction,
                InvocationContext {
                    invocation_id: revision,
                    subscription_id: String::new(),
                    document_id: "file:///workspace/function.sk".into(),
                    document_revision: revision,
                    expansion: None,
                    syntax_context: 0,
                },
                ExpressionParseRequest {
                    source: &source,
                    range: TextRange::new(0, text.len()),
                    expected_types: vec![ExpressionExpectedType {
                        class_name: ClassName("java.lang.Object".into()),
                        plural: true,
                    }],
                    context: ExpressionParseContext::default(),
                },
                ExpressionParserConfig::default(),
            );
            if oversized {
                assert!(
                    matches!(
                        result,
                        Err(parser_wasm::host::HostError::ExpressionParser(
                            skript_parser::ExpressionParseError::Environment { ref message }
                        )) if message == &parser_wasm::host::HostError::GeneratedOutputQuotaExceeded { limit: LIMIT }.to_string()
                    ),
                    "{field}: {result:?}"
                );
                assert!(
                    transaction.read_write_set().unwrap().writes.is_empty(),
                    "{field}"
                );
                let mut invocation = transaction.begin_invocation("test.function-addon").unwrap();
                for key in ["accepted-function", "invalid-function"] {
                    assert!(
                        invocation
                            .get(
                                StateScope::Parse,
                                NamespaceVisibility::Private,
                                "function-state",
                                key
                            )
                            .unwrap()
                            .is_none(),
                        "{field}: {key}"
                    );
                }
                invocation.rollback();
            } else {
                assert!(
                    result.unwrap().matches.selected.is_some(),
                    "host must remain usable after {field}"
                );
            }
            transaction.cancel().unwrap();
        }
    }
}

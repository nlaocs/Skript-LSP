use parser_wasm::ParseTransaction;
use parser_wasm::host::{
    HostConfig, HostError, InvocationContext, ParserHost, RuntimeProfile, WasmExpressionParseResult,
};
use skript_parser::{
    ExpressionExpectedType, ExpressionNode, ExpressionNodeKind, ExpressionParseContext,
    ExpressionParseRequest, ExpressionParserConfig, MappedSource, TextRange,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use syntaxes::{Catalog, ClassName, Multiplicity};

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));
const ADDON_A: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/expression-data-addon-a.wasm"
));
const ADDON_B: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/expression-data-addon-b.wasm"
));

const VARIABLE_SCHEMA_ID: &str = "nlaocs.skript.variable";
const EDITED_VARIABLE_JSON: &str =
    r#"{"scope":"global","name":[{"kind":"text","text":"wallet::balances::*"}]}"#;
const CORE_VARIABLE_METADATA_KEY: &str = "nlaocs.core-library/expression.capability.key-provider";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn catalog() -> Arc<Catalog> {
    Arc::new(
        ssg::load(fixture())
            .expect("core2.15.4 fixture must load")
            .catalog()
            .clone(),
    )
}

fn host_with_catalog(syntax_catalog: Arc<Catalog>) -> ParserHost {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(syntax_catalog),
            runtime_profile: RuntimeProfile {
                skript_version: Some("2.15.4".to_owned()),
                ..RuntimeProfile::default()
            },
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    host.load_addon(ADDON_A)
        .expect("expression-data addon A must load");
    host.load_addon(ADDON_B)
        .expect("expression-data addon B must load");
    host
}

fn host() -> ParserHost {
    host_with_catalog(catalog())
}

fn context(revision: u64) -> InvocationContext {
    InvocationContext {
        invocation_id: revision,
        subscription_id: String::new(),
        document_id: "file:///workspace/expression-data.sk".to_owned(),
        document_revision: revision,
        expansion: None,
        syntax_context: 7,
    }
}

fn parse(
    host: &mut ParserHost,
    transaction: &ParseTransaction,
    text: &str,
    revision: u64,
    plural: bool,
) -> Result<WasmExpressionParseResult, HostError> {
    let source = MappedSource::identity(text);
    host.parse_expression_in_parse(
        transaction,
        context(revision),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![ExpressionExpectedType {
                class_name: ClassName("java.lang.Object".to_owned()),
                plural,
            }],
            context: ExpressionParseContext {
                syntax_context: 7,
                ..ExpressionParseContext::default()
            },
        },
        ExpressionParserConfig::default(),
    )
}

fn selected(result: WasmExpressionParseResult) -> ExpressionNode {
    if result.matches.selected.is_none() {
        panic!(
            "expression fixture must select a candidate: matches={:#?}, failures={:#?}, calls={:#?}",
            result.matches, result.failures, result.calls
        );
    }
    result.matches.selected.unwrap().node
}

fn call_index(result: &WasmExpressionParseResult, component: &str, subscription: &str) -> usize {
    result
        .calls
        .iter()
        .position(|call| call.component_id == component && call.subscription_id == subscription)
        .unwrap_or_else(|| panic!("missing call {component}/{subscription}: {result:#?}"))
}

#[test]
fn core_variable_public_data_flows_through_two_addons_and_parent_capture() {
    let mut host = host();
    let transaction = host
        .begin_parse(
            "file:///workspace",
            "file:///workspace/expression-data.sk",
            1,
        )
        .unwrap();
    let result = parse(&mut host, &transaction, "{_balances::*}", 1, true).unwrap();
    let node = result
        .matches
        .selected
        .as_ref()
        .expect("expression fixture must select a candidate")
        .node
        .clone();

    assert!(matches!(node.kind, ExpressionNodeKind::Variable { .. }));
    assert_eq!(
        node.return_type.as_ref().map(ClassName::as_str),
        Some("java.lang.Long")
    );
    assert_eq!(node.multiplicity, Some(Multiplicity::Multiple));
    assert_eq!(node.children.len(), 0);
    assert_eq!(node.public_data.len(), 1);
    assert_eq!(node.public_data[0].schema_id, VARIABLE_SCHEMA_ID);
    assert_eq!(node.public_data[0].schema_version, 1);
    assert_eq!(node.public_data[0].json, EDITED_VARIABLE_JSON);
    let public_json: serde_json::Value = serde_json::from_str(&node.public_data[0].json).unwrap();
    assert!(public_json.get("type").is_none());
    assert!(public_json.get("multiplicity").is_none());
    assert_eq!(
        node.metadata
            .get(CORE_VARIABLE_METADATA_KEY)
            .map(String::as_str),
        Some("true")
    );

    assert!(result.failures.iter().any(|failure| {
        failure.component_id == "test.expression-data-a"
            && failure.subscription_id == "expression-data.a.metadata"
            && matches!(&failure.error, HostError::InvalidHookOutput { .. })
            && failure
                .error
                .to_string()
                .contains("cannot write metadata owned by nlaocs.core-library")
    }));
    let core_index = call_index(&result, "nlaocs.core-library", "core.expression-candidates");
    let edit_index = call_index(&result, "test.expression-data-a", "expression-data.a.edit");
    let metadata_index = call_index(
        &result,
        "test.expression-data-a",
        "expression-data.a.metadata",
    );
    let observe_index = call_index(
        &result,
        "test.expression-data-b",
        "expression-data.b.observe",
    );
    assert!(core_index < edit_index);
    assert!(edit_index < metadata_index);
    assert!(metadata_index < observe_index);

    let parent_result = parse(&mut host, &transaction, "reversed {_balances::*}", 1, true).unwrap();
    let parent = parent_result
        .matches
        .selected
        .as_ref()
        .expect("parent expression must select a candidate")
        .node
        .clone();
    let capture = parent
        .parsed_captures()
        .into_iter()
        .next()
        .expect("parent expression must retain its variable capture");
    let summary = capture
        .result
        .summary
        .expect("parent capture summary must be retained");
    assert_eq!(summary.public_data.len(), 1);
    assert_eq!(summary.public_data[0].json, EDITED_VARIABLE_JSON);
    assert_eq!(
        summary.return_type.as_ref().map(ClassName::as_str),
        Some("java.lang.Long")
    );
    assert_eq!(
        parent.return_type.as_ref().map(ClassName::as_str),
        Some("java.lang.Long")
    );
    transaction.cancel().unwrap();
}

#[test]
fn grouped_and_interpolated_variables_keep_data_on_the_underlying_node() {
    let mut host = host();
    let transaction = host
        .begin_parse(
            "file:///workspace",
            "file:///workspace/expression-data.sk",
            2,
        )
        .unwrap();

    let grouped_parent = selected(
        parse(
            &mut host,
            &transaction,
            "reversed ({_balances::*})",
            2,
            true,
        )
        .unwrap(),
    );
    let grouped = &grouped_parent.children[0];
    assert!(matches!(grouped.kind, ExpressionNodeKind::Grouped));
    assert!(grouped.public_data.is_empty());
    assert_eq!(grouped.children.len(), 1);
    let grouped_variable = &grouped.children[0];
    assert!(matches!(
        grouped_variable.kind,
        ExpressionNodeKind::Variable { .. }
    ));
    assert_eq!(grouped_variable.public_data.len(), 1);
    assert_eq!(grouped_variable.multiplicity, Some(Multiplicity::Multiple));

    let text = "{data::%{_first}%::%{_second}%}";
    let interpolated = selected(parse(&mut host, &transaction, text, 2, true).unwrap());
    assert!(matches!(
        interpolated.kind,
        ExpressionNodeKind::Variable { .. }
    ));
    assert_eq!(interpolated.public_data.len(), 1);
    assert_eq!(interpolated.multiplicity, Some(Multiplicity::Single));
    assert_eq!(interpolated.children.len(), 2);
    let first_start = text.find("{_first}").unwrap();
    let second_start = text.find("{_second}").unwrap();
    assert_eq!(
        interpolated.children[0].span.mapped.virtual_range,
        TextRange::new(first_start, first_start + "{_first}".len())
    );
    assert_eq!(
        interpolated.children[1].span.mapped.virtual_range,
        TextRange::new(second_start, second_start + "{_second}".len())
    );
    let public_json: serde_json::Value =
        serde_json::from_str(&interpolated.public_data[0].json).unwrap();
    assert_eq!(public_json["scope"], "global");
    assert_eq!(
        public_json["name"],
        serde_json::json!([
            {"kind": "text", "text": "data::"},
            {"kind": "expression", "childIndex": 0},
            {"kind": "text", "text": "::"},
            {"kind": "expression", "childIndex": 1}
        ])
    );
    transaction.cancel().unwrap();
}

#[test]
fn invalid_public_data_is_rejected_before_addon_state_merges() {
    let mut host = host();
    let transaction = host
        .begin_parse(
            "file:///workspace",
            "file:///workspace/expression-data.sk",
            3,
        )
        .unwrap();
    for (text, subscription, message) in [
        (
            "{invalid-json::*}",
            "expression-data.a.invalid",
            "must be a JSON object",
        ),
        (
            "{repeated-schema::*}",
            "expression-data.a.invalid",
            "is repeated",
        ),
    ] {
        let result = parse(&mut host, &transaction, text, 3, true).unwrap();
        let node = result
            .matches
            .selected
            .as_ref()
            .expect("invalid candidate output must fall back to the core candidate")
            .node
            .clone();
        assert_eq!(
            node.return_type.as_ref().map(ClassName::as_str),
            Some("java.lang.Object")
        );
        assert_eq!(node.public_data.len(), 1);
        assert!(
            node.public_data[0]
                .json
                .starts_with("{\"scope\":\"global\"")
        );
        assert!(result.failures.iter().any(|failure| {
            failure.component_id == "test.expression-data-a"
                && failure.subscription_id == subscription
                && matches!(&failure.error, HostError::InvalidHookOutput { .. })
                && failure.error.to_string().contains(message)
        }));
        assert!(
            transaction
                .read_write_set()
                .unwrap()
                .writes
                .iter()
                .all(|write| { !matches!(write.key.as_str(), "invalid-json" | "repeated-schema") })
        );
    }
    transaction.cancel().unwrap();
}

#[test]
fn rejected_candidate_rolls_back_and_later_parse_still_sees_clean_state() {
    let mut host = host();
    let transaction = host
        .begin_parse(
            "file:///workspace",
            "file:///workspace/expression-data.sk",
            4,
        )
        .unwrap();
    let rejected = parse(&mut host, &transaction, "{rollback::*}", 4, true).unwrap();
    assert!(rejected.matches.selected.is_none());
    assert!(
        transaction
            .read_write_set()
            .unwrap()
            .writes
            .iter()
            .all(|write| write.key != "rejected-candidate")
    );

    let accepted = parse(&mut host, &transaction, "{_balances::*}", 4, true).unwrap();
    let node = selected(accepted);
    assert_eq!(
        node.return_type.as_ref().map(ClassName::as_str),
        Some("java.lang.Long")
    );
    assert_eq!(node.public_data[0].json, EDITED_VARIABLE_JSON);
    transaction.cancel().unwrap();
}

#[test]
fn parse_result_summary_public_data_is_validated() {
    let mut host = host();
    let transaction = host
        .begin_parse(
            "file:///workspace",
            "file:///workspace/expression-data.sk",
            5,
        )
        .unwrap();
    let error = parse(&mut host, &transaction, "{invalid-summary::*}", 5, true)
        .expect_err("invalid parse-result summary must fail the recursive request");
    let message = error.to_string();
    assert!(message.contains("must be a JSON object"), "{message}");
    transaction.cancel().unwrap();
}

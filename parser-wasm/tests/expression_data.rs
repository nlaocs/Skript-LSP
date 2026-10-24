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

    assert!(result.failures.is_empty(), "{result:#?}");
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

// Public records are editable data, not owner credentials. Validate the entire
// envelope and each expected schema; native authority is checked separately below.
fn assert_public_records<'a>(
    entries: impl IntoIterator<Item = (&'a str, u32, &'a str)>,
    expected_schemas: &[&str],
) -> std::collections::BTreeMap<&'a str, serde_json::Value> {
    let mut records = std::collections::BTreeMap::new();
    for (schema, version, json) in entries {
        assert_eq!(version, 1, "{schema}: unsupported public schema version");
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(
            value.is_object(),
            "{schema}: public data must remain an object"
        );
        assert!(
            records.insert(schema, value).is_none(),
            "{schema}: duplicate schema"
        );
    }
    assert_eq!(
        records.keys().copied().collect::<Vec<_>>(),
        expected_schemas.to_vec()
    );
    records
}

#[test]
fn grouped_and_interpolated_variables_keep_data_on_the_underlying_node() {
    const IDENTITY: &str = "nlaocs.skript.variable-identity";
    const INTERPOLATION: &str = "nlaocs.skript.interpolation";
    const CORE: &str = "nlaocs.core-library";
    const VARIABLE_READ: &str = "nlaocs.core-library/skript.semantic.variable-read";

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
    assert!(
        grouped.public_data.is_empty(),
        "the Grouped wrapper must not duplicate variable records"
    );
    assert_eq!(grouped.children.len(), 1);
    let grouped_variable = &grouped.children[0];
    assert!(matches!(
        grouped_variable.kind,
        ExpressionNodeKind::Variable { .. }
    ));
    // Addon A deliberately replaces this variable's public vector; Addon B sees
    // that replacement. The wrapper still delegates to the actual underlying node.
    let grouped_records = assert_public_records(
        grouped_variable.public_data.iter().map(|record| {
            (
                record.schema_id.as_str(),
                record.schema_version,
                record.json.as_str(),
            )
        }),
        &[VARIABLE_SCHEMA_ID],
    );
    assert_eq!(
        grouped_records[VARIABLE_SCHEMA_ID],
        serde_json::from_str::<serde_json::Value>(EDITED_VARIABLE_JSON).unwrap()
    );
    assert_eq!(grouped_variable.multiplicity, Some(Multiplicity::Multiple));
    assert!(grouped_variable.children.is_empty());
    assert!(
        grouped_parent.public_data.iter().all(|record| {
            record.schema_id != VARIABLE_SCHEMA_ID && record.schema_id != IDENTITY
        }),
        "the registered parent must not own its captured variable's data"
    );
    let grouped_capture = grouped_parent.parsed_captures().into_iter().next().unwrap();
    let grouped_summary = grouped_capture.result.summary.unwrap();
    let summary_records = assert_public_records(
        grouped_summary.public_data.iter().map(|record| {
            (
                record.schema_id.as_str(),
                record.schema_version,
                record.json.as_str(),
            )
        }),
        &[VARIABLE_SCHEMA_ID],
    );
    assert_eq!(
        summary_records, grouped_records,
        "capture summary must delegate the underlying variable data"
    );

    let text = "{data::%{_first}%::%{_second}%}";
    let interpolated = selected(parse(&mut host, &transaction, text, 2, true).unwrap());
    assert!(matches!(
        interpolated.kind,
        ExpressionNodeKind::Variable { .. }
    ));
    assert_eq!(interpolated.multiplicity, Some(Multiplicity::Single));
    assert_eq!(interpolated.children.len(), 2);
    let records = assert_public_records(
        interpolated.public_data.iter().map(|record| {
            (
                record.schema_id.as_str(),
                record.schema_version,
                record.json.as_str(),
            )
        }),
        &[INTERPOLATION, VARIABLE_SCHEMA_ID, IDENTITY],
    );
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
    assert_eq!(
        records[VARIABLE_SCHEMA_ID],
        serde_json::json!({
            "scope": "global",
            "name": [
                {"kind": "text", "text": "data::"},
                {"kind": "expression", "childIndex": 0},
                {"kind": "text", "text": "::"},
                {"kind": "expression", "childIndex": 1}
            ]
        })
    );
    assert_eq!(
        records[IDENTITY],
        serde_json::json!({
            "scope": "global", "resolution": "dynamic", "list": false, "casePolicy": "unknown"
        }),
        "dynamic templates must not acquire a guessed name, alias or canonical key"
    );
    assert_eq!(
        records[INTERPOLATION],
        serde_json::json!({
            "expressions": [
                {"childIndex": 0, "opening": first_start - 1, "closing": first_start + "{_first}".len()},
                {"childIndex": 1, "opening": second_start - 1, "closing": second_start + "{_second}".len()}
            ]
        })
    );
    assert!(
        !interpolated.metadata.contains_key(VARIABLE_READ),
        "dynamic name evaluation is not a static variable-read witness"
    );
    for (child, name) in interpolated.children.iter().zip(["first", "second"]) {
        assert!(matches!(child.kind, ExpressionNodeKind::Variable { .. }));
        assert!(child.children.is_empty());
        let child_records = assert_public_records(
            child.public_data.iter().map(|record| {
                (
                    record.schema_id.as_str(),
                    record.schema_version,
                    record.json.as_str(),
                )
            }),
            &[VARIABLE_SCHEMA_ID, IDENTITY],
        );
        assert_eq!(
            child_records[VARIABLE_SCHEMA_ID],
            serde_json::json!({
                "scope": "local", "name": [{"kind": "text", "text": name}]
            })
        );
        assert_eq!(
            child_records[IDENTITY],
            serde_json::json!({
                "scope": "local", "resolution": "static", "name": name,
                "aliasKey": name, "list": false, "casePolicy": "unknown",
                "nameRange": {"start": 2, "end": 2 + name.len(), "grammar": "ascii-identifier-v1"}
            })
        );
        let start = child.span.mapped.virtual_range.start + 2;
        assert_eq!(
            text.get(start..start + name.len()), Some(name),
            "identity range is relative to the underlying variable, not its interpolation parent"
        );
        let read: serde_json::Value = serde_json::from_str(
            child
                .metadata
                .get(VARIABLE_READ)
                .expect("the actual Core scalar read boundary must remain owned"),
        )
        .unwrap();
        assert_eq!(read["version"], 1);
        assert_eq!(
            read["requestedTypes"],
            serde_json::json!(["java.lang.Object"])
        );
        assert_eq!(read["mayRefreshPlayer"], true);
    }
    transaction.cancel().unwrap();

    // A schema name alone cannot authenticate these facts. Exercise publication
    // through a real Core-only document and the host's immutable native registry.
    let mut published_host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            runtime_profile: RuntimeProfile {
                skript_version: Some("2.15.4".into()),
                ..RuntimeProfile::default()
            },
            ..HostConfig::default()
        },
    )
    .unwrap();
    let source = format!("on load:\n    send {text} to console\n");
    let published = published_host
        .parse_document(
            parser_wasm::DocumentParseRequest::new(
                "file:///workspace",
                "file:///workspace/published-expression-data.sk",
                3,
                source,
            ),
            parser_wasm::DocumentParserConfig::default(),
        )
        .unwrap();
    assert!(published.component_failures.is_empty());
    assert!(published.syntax.diagnostics.is_empty());
    let nodes = published
        .ast
        .nodes
        .iter()
        .filter(|node| {
            node.text == text
                && node
                    .summary
                    .as_ref()
                    .is_some_and(|summary| summary.kind == "variable")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        nodes.len(),
        1,
        "published data must belong to one actual underlying variable node"
    );
    let node = nodes[0];
    let summary = node.summary.as_ref().unwrap();
    let published_records = assert_public_records(
        summary.public_data.iter().map(|record| {
            (
                record.schema_id.as_str(),
                record.schema_version,
                record.json.as_str(),
            )
        }),
        &[INTERPOLATION, VARIABLE_SCHEMA_ID, IDENTITY],
    );
    assert_eq!(published_records, records);
    let witnesses = published.semantic_witnesses();
    assert!(
        witnesses.native_valid(node),
        "published native proof rejected node{}; retained calls={:?}; native node receipt evidence={:?}",
        node.id,
        published.syntax_calls,
        published
            .ast
            .nodes
            .iter()
            .map(|current| {
                let receipts = current
                    .metadata
                    .iter()
                    .chain(
                        current
                            .summary
                            .iter()
                            .flat_map(|summary| summary.metadata.iter()),
                    )
                    .filter(|entry| {
                        entry.owner_component_id.is_none()
                            && entry.key == "parser.semantic-writer-receipt.v1"
                    })
                    .map(|entry| entry.value.as_str())
                    .collect::<Vec<_>>();
                (
                    current.id,
                    current.text.as_str(),
                    witnesses.native_valid(current),
                    receipts,
                )
            })
            .collect::<Vec<_>>()
    );
    assert!(published.ast.nodes.len() > node.children.len() + 1);
    for current in &published.ast.nodes {
        assert!(
            witnesses.native_valid(current),
            "healthy published node{} ({}) must retain its native proof alongside the interpolated variable",
            current.id,
            current.text
        );
    }
    assert_eq!(witnesses.unique_issuer(node), Some(CORE));
    assert!(witnesses.provider_valid(node, CORE));
    let copied_node = (*node).clone();
    assert!(
        !witnesses.native_valid(&copied_node),
        "copying identical public records must not mint native authority"
    );
    assert!(!witnesses.provider_valid(&copied_node, CORE));
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
        let error = parse(&mut host, &transaction, text, 3, true)
            .expect_err("failed transformation must not fall back to a core candidate");
        let fault = skript_parser::extension_fault(&error).expect("typed execution cause");
        assert_eq!(fault.component_id, "test.expression-data-a");
        assert_eq!(fault.subscription_id, subscription);
        assert_eq!(fault.code, "parser.extension-invalid-output");
        assert!(fault.message.contains(message), "{fault:?}");
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

#[test]
fn failed_late_validator_discards_earlier_transformation_instead_of_adopting_it() {
    let mut host = host();
    let transaction = host
        .begin_parse(
            "file:///workspace",
            "file:///workspace/expression-data.sk",
            6,
        )
        .unwrap();
    let error = parse(&mut host, &transaction, "{spoof-core-metadata::*}", 6, true)
        .expect_err("no alternate success after a fault");
    let fault = skript_parser::extension_fault(&error).expect("typed cause");
    assert_eq!(fault.subscription_id, "expression-data.a.metadata");
    assert!(
        fault
            .message
            .contains("cannot write metadata owned by nlaocs.core-library")
    );
    assert!(transaction.read_write_set().unwrap().writes.is_empty());
    let clean = parse(&mut host, &transaction, "{_balances::*}", 6, true)
        .expect("a different input remains independently parseable");
    assert_eq!(
        selected(clean).return_type.as_ref().map(ClassName::as_str),
        Some("java.lang.Long")
    );
    transaction.cancel().unwrap();
}

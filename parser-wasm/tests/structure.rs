use parser_wasm::host::{HostConfig, InvocationContext, ParserHost};
use skript_parser::{
    ExpressionParseContext, MappedSource, ParsedCaptureValue, RawTreeOptions, StructureBody,
    StructureDocumentNode, StructureEntryValue, StructureParseRequest, StructureParserConfig,
    parse_raw_tree,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use syntaxes::Catalog;

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn catalog() -> Arc<Catalog> {
    Arc::new(ssg::load(fixture()).unwrap().catalog().clone())
}

fn context(revision: u64) -> InvocationContext {
    InvocationContext {
        invocation_id: revision,
        subscription_id: String::new(),
        document_id: "file:///workspace/structure.sk".to_owned(),
        document_revision: revision,
        expansion: None,
        syntax_context: 0,
    }
}

fn parse(
    host: &mut ParserHost,
    revision: u64,
    input: &str,
) -> parser_wasm::WasmStructureParseResult {
    let transaction = host
        .begin_parse(
            "file:///workspace",
            "file:///workspace/structure.sk",
            revision,
        )
        .unwrap();
    let source = MappedSource::identity(input);
    let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
    let result = host
        .parse_structures_in_parse(
            &transaction,
            context(revision),
            StructureParseRequest {
                source: &source,
                tree: &tree,
                context: ExpressionParseContext::default(),
            },
            StructureParserConfig::default(),
        )
        .unwrap();
    transaction.cancel().unwrap();
    result
}

fn selected(result: &parser_wasm::WasmStructureParseResult) -> &skript_parser::StructureCandidate {
    let StructureDocumentNode::Structure(matches) = &result.document.roots[0] else {
        panic!("first top-level node must use the Structure pipeline");
    };
    matches.selected.as_ref().expect("Structure must match")
}

#[test]
fn struct_event_delegates_event_capture_and_body_semantics_to_core_library() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            ..HostConfig::default()
        },
    )
    .unwrap();
    let result = parse(
        &mut host,
        1,
        "on dummy fixture event:\n    dummy effect registered through wrapper\n",
    );
    let candidate = selected(&result);

    assert_eq!(
        candidate.element_class.as_ref().map(|value| value.as_str()),
        Some("ch.njol.skript.structures.StructEvent")
    );
    assert!(
        candidate
            .parsed_captures
            .iter()
            .any(|capture| { matches!(capture.result.value, Some(ParsedCaptureValue::Event(_))) })
    );
    assert!(
        result.calls.iter().any(|call| {
            call.component_id == "nlaocs.core-library"
                && call.subscription_id == "core.structure-semantics"
        }),
        "Structure hook must run: {:#?}",
        result.calls
    );
    let StructureBody::Trigger(body) = &candidate.body else {
        panic!("StructEvent must select the trigger body parser through WASM: {candidate:#?}");
    };
    assert_eq!(body.len(), 1);
    assert!(result.document.diagnostics.is_empty());
    assert!(result.effects.context_updates.iter().any(|update| {
        update.key == "parser.event-classes"
            && update
                .value
                .as_deref()
                .is_some_and(|value| String::from_utf8_lossy(value).contains("DummyEvent"))
    }));
    assert!(result.calls.iter().any(|call| {
        call.component_id == "nlaocs.core-library"
            && call.subscription_id == "core.structure-semantics"
    }));
}

#[test]
fn function_and_command_headers_reach_their_specialized_wasm_handlers() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            ..HostConfig::default()
        },
    )
    .unwrap();
    let function = parse(
        &mut host,
        2,
        "function fixture():\n    dummy effect registered through wrapper\n",
    );
    let function = selected(&function);
    assert!(
        matches!(function.body, StructureBody::Trigger(_)),
        "{function:#?}"
    );
    assert_eq!(
        function
            .metadata
            .get("nlaocs.core-library/semantic-mode")
            .map(String::as_str),
        Some("function-structure")
    );

    let command = parse(
        &mut host,
        3,
        "command /fixture:\n    trigger:\n        dummy effect registered through wrapper\n",
    );
    let command = selected(&command);
    assert!(matches!(command.body, StructureBody::Entries(_)));
    assert_eq!(
        command
            .metadata
            .get("nlaocs.core-library/semantic-mode")
            .map(String::as_str),
        Some("command-structure")
    );
}

#[test]
fn addon_defined_entry_data_remains_visible_to_wasm_and_lsp_consumers() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            ..HostConfig::default()
        },
    )
    .unwrap();
    let result = parse(
        &mut host,
        4,
        "custom event \"fixture\":\n    patterns: fixture pattern\n",
    );
    let candidate = selected(&result);
    let StructureBody::Entries(entries) = &candidate.body else {
        panic!("custom event must retain its EntryValidator output");
    };
    assert!(entries.iter().any(|entry| {
        entry.key == "patterns"
            && matches!(
                &entry.value,
                StructureEntryValue::Unknown(value) if value == "fixture pattern"
            )
    }));
}

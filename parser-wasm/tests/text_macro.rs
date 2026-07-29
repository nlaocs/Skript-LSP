use parser_wasm::host::{
    HookDecision, HostConfig, HostError, InvocationContext, ParserHost, TextMacroRequest,
};
use skript_parser::{MappedSource, OriginKind, TextRange};

const CORE_LIBRARY: &[u8] = include_bytes!("../../artifacts/core-library.wasm");
const TEXT_MACRO_ADDON: &[u8] = include_bytes!("../../artifacts/text-macro-addon.wasm");
const COMPONENT_ID: &str = "nlaocs.test.text-macro";

fn context(revision: u64) -> InvocationContext {
    InvocationContext {
        invocation_id: revision,
        subscription_id: String::new(),
        document_id: "file:///workspace/test.sk".to_owned(),
        document_revision: revision,
        expansion: None,
        syntax_context: 0,
    }
}

fn host(config: HostConfig) -> ParserHost {
    let mut host = ParserHost::new(CORE_LIBRARY, config).expect("CoreLibrary must initialize");
    host.load_addon(TEXT_MACRO_ADDON)
        .expect("text macro addon must initialize");
    host
}

fn written_keys(transaction: &parser_wasm::state::ParseTransaction) -> Vec<String> {
    transaction
        .read_write_set()
        .expect("state access set must remain available")
        .writes
        .into_iter()
        .map(|entry| entry.key)
        .collect()
}

#[test]
fn composes_macros_by_priority_and_maps_nested_expansions() {
    let mut host = host(HostConfig::default());
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 1)
        .expect("parse must begin");
    let result = host
        .expand_text_in_parse(
            &transaction,
            TextMacroRequest {
                context: context(1),
                source: MappedSource::identity("alpha"),
            },
        )
        .expect("text macros must run");

    assert_eq!(result.source.virtual_source(), "二段目");
    assert!(result.failures.is_empty());
    assert_eq!(
        result
            .calls
            .iter()
            .map(|call| call.subscription_id.as_str())
            .collect::<Vec<_>>(),
        ["text.first", "text.second"]
    );
    assert!(result.calls.iter().all(|call| call.accepted));
    let first = result.calls[0].expansion.expect("first edit expands");
    let second = result.calls[1].expansion.expect("second edit expands");
    let backtrace = result
        .source
        .expansion_backtrace(second)
        .expect("nested expansion must have a backtrace");
    assert_eq!(
        backtrace
            .iter()
            .map(|expansion| expansion.id)
            .collect::<Vec<_>>(),
        [second, first],
    );
    assert_eq!(backtrace[0].component.as_str(), COMPONENT_ID);
    assert_eq!(backtrace[0].hook.as_str(), "text.second");
    assert_eq!(backtrace[0].call_site.expansion, Some(first));
    assert_eq!(backtrace[1].component.as_str(), COMPONENT_ID);
    assert_eq!(backtrace[1].hook.as_str(), "text.first");
    assert_eq!(backtrace[1].call_site.expansion, None);
    for call in &result.calls {
        assert!(call.state_accesses.reads.is_empty());
        assert_eq!(
            call.state_accesses
                .writes
                .iter()
                .map(|entry| entry.key.as_str())
                .collect::<Vec<_>>(),
            [call.subscription_id.as_str()],
        );
    }
    assert_eq!(result.effects.diagnostics.len(), 1);
    let diagnostic = &result.effects.diagnostics[0];
    assert_eq!(diagnostic.code, "fixture.generated-source");
    assert_eq!(diagnostic.span.virtual_range.start, 0);
    assert_eq!(diagnostic.span.virtual_range.end, 9);
    assert_eq!(diagnostic.span.origins.len(), 1);
    assert_eq!(diagnostic.span.origins[0].original_range.start, 0);
    assert_eq!(diagnostic.span.origins[0].original_range.end, 5);
    assert_eq!(
        diagnostic.span.origins[0].expansion,
        Some(u64::from(first.get()))
    );
    assert_eq!(diagnostic.related.len(), 1);
    assert_eq!(diagnostic.related[0].span.virtual_range.start, 9);
    assert_eq!(diagnostic.related[0].span.virtual_range.end, 9);
    assert!(
        diagnostic.related[0]
            .span
            .origins
            .iter()
            .all(|origin| origin.original_range.start <= 5)
    );
    assert_eq!(result.effects.parse_requests.len(), 1);
    let parse_request = &result.effects.parse_requests[0];
    assert_eq!(parse_request.span.virtual_range.start, 0);
    assert_eq!(parse_request.span.virtual_range.end, 9);
    assert_eq!(parse_request.span.origins.len(), 1);
    assert_eq!(parse_request.span.origins[0].original_range.start, 0);
    assert_eq!(parse_request.span.origins[0].original_range.end, 5);
    assert_eq!(
        parse_request.span.origins[0].expansion,
        Some(u64::from(first.get()))
    );
    let generated = result
        .source
        .map_range(TextRange::new(0, "二段目".len()))
        .expect("generated range must map")
        .primary_origin()
        .expect("generated range has an origin");
    assert_eq!(generated.original_range, TextRange::new(0, 5));
    assert_eq!(generated.kind, OriginKind::Replaced);
    assert_eq!(generated.expansion, Some(second));
    assert_eq!(written_keys(&transaction), ["text.first", "text.second"]);
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn rejects_invalid_utf8_edits_without_committing_their_state() {
    let mut host = host(HostConfig::default());
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 2)
        .expect("parse must begin");
    let original = "日本 invalid";
    let result = host
        .expand_text_in_parse(
            &transaction,
            TextMacroRequest {
                context: context(2),
                source: MappedSource::identity(original),
            },
        )
        .expect("one invalid macro must not abort later macros");

    assert_eq!(result.source.virtual_source(), original);
    assert_eq!(result.failures.len(), 1);
    assert!(matches!(
        result.failures[0].error,
        HostError::InvalidTextMacroOutput { .. }
    ));
    assert!(!result.calls[0].accepted);
    assert!(result.calls[1].accepted);
    assert_eq!(written_keys(&transaction), ["text.second"]);
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn rejects_invalid_diagnostic_spans_without_committing_their_state() {
    let mut host = host(HostConfig::default());
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 9)
        .expect("parse must begin");
    let original = "日本 bad-diagnostic";
    let result = host
        .expand_text_in_parse(
            &transaction,
            TextMacroRequest {
                context: context(9),
                source: MappedSource::identity(original),
            },
        )
        .expect("one invalid diagnostic must not abort later macros");

    assert_eq!(result.source.virtual_source(), original);
    assert!(result.effects.diagnostics.is_empty());
    assert_eq!(result.failures.len(), 1);
    assert!(matches!(
        result.failures[0].error,
        HostError::InvalidTextMacroOutput { .. }
    ));
    assert!(!result.calls[0].accepted);
    assert!(result.calls[1].accepted);
    assert_eq!(written_keys(&transaction), ["text.second"]);
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn rejects_invalid_parse_request_spans_without_committing_their_state() {
    let mut host = host(HostConfig::default());
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 11)
        .expect("parse must begin");
    let original = "日本 bad-request-span";
    let result = host
        .expand_text_in_parse(
            &transaction,
            TextMacroRequest {
                context: context(11),
                source: MappedSource::identity(original),
            },
        )
        .expect("one invalid parse request must not abort later macros");

    assert_eq!(result.source.virtual_source(), original);
    assert!(result.effects.parse_requests.is_empty());
    assert_eq!(result.failures.len(), 1);
    assert!(matches!(
        result.failures[0].error,
        HostError::InvalidTextMacroOutput { .. }
    ));
    assert!(!result.calls[0].accepted);
    assert!(result.calls[1].accepted);
    assert_eq!(written_keys(&transaction), ["text.second"]);
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn reject_rolls_back_prior_text_and_state_for_the_pipeline() {
    let mut host = host(HostConfig::default());
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 3)
        .expect("parse must begin");
    let result = host
        .expand_text_in_parse(
            &transaction,
            TextMacroRequest {
                context: context(3),
                source: MappedSource::identity("reject"),
            },
        )
        .expect("typed rejection is a successful pipeline result");

    let HookDecision::Reject(rejection) = &result.decision else {
        panic!("fixture must reject");
    };
    assert_eq!(rejection.diagnostics.len(), 1);
    let diagnostic = &rejection.diagnostics[0];
    assert_eq!(diagnostic.code, "fixture.unclosed-delimiter");
    assert_eq!(diagnostic.span.virtual_range.start, 6);
    assert_eq!(diagnostic.span.virtual_range.end, 6);
    assert_eq!(diagnostic.span.origins.len(), 1);
    assert_eq!(diagnostic.span.origins[0].original_range.start, 6);
    assert_eq!(diagnostic.span.origins[0].original_range.end, 6);
    assert_eq!(diagnostic.span.origins[0].expansion, None);
    assert_eq!(diagnostic.related.len(), 1);
    assert_eq!(diagnostic.related[0].span.origins.len(), 1);
    assert_eq!(
        diagnostic.related[0].span.origins[0].original_range.start,
        0
    );
    assert_eq!(diagnostic.related[0].span.origins[0].original_range.end, 1);
    assert_eq!(result.source.virtual_source(), "reject");
    assert_eq!(result.calls.len(), 1);
    assert!(!result.calls[0].accepted);
    assert!(written_keys(&transaction).is_empty());
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn late_reject_removes_rolled_back_expansion_references() {
    let mut host = host(HostConfig::default());
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 10)
        .expect("parse must begin");
    let original = "alpha late-stop";
    let result = host
        .expand_text_in_parse(
            &transaction,
            TextMacroRequest {
                context: context(10),
                source: MappedSource::identity(original),
            },
        )
        .expect("late rejection is a successful pipeline result");

    assert_eq!(result.source.virtual_source(), original);
    assert!(result.source.expansions().is_empty());
    assert_eq!(result.calls.len(), 2);
    assert!(
        result
            .calls
            .iter()
            .all(|call| !call.accepted && call.expansion.is_none())
    );
    let HookDecision::Reject(rejection) = &result.decision else {
        panic!("second macro must reject");
    };
    assert_eq!(rejection.diagnostics.len(), 1);
    let diagnostic = &rejection.diagnostics[0];
    assert_eq!(diagnostic.code, "fixture.late-rejection");
    assert_eq!(diagnostic.span.origins.len(), 1);
    assert_eq!(diagnostic.span.origins[0].original_range.start, 0);
    assert_eq!(diagnostic.span.origins[0].original_range.end, 5);
    assert_eq!(diagnostic.span.origins[0].expansion, None);
    assert_eq!(diagnostic.related.len(), 1);
    assert_eq!(diagnostic.related[0].span.origins.len(), 1);
    assert_eq!(
        diagnostic.related[0].span.origins[0].original_range.start,
        original.len() as u64
    );
    assert_eq!(
        diagnostic.related[0].span.origins[0].original_range.end,
        original.len() as u64
    );
    assert_eq!(diagnostic.related[0].span.origins[0].expansion, None);
    assert!(result.effects.context_updates.is_empty());
    assert!(result.effects.parse_requests.is_empty());
    assert!(written_keys(&transaction).is_empty());
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn explicit_anchor_maps_inserted_text_to_the_requested_call_site() {
    let mut host = host(HostConfig::default());
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 4)
        .expect("parse must begin");
    let result = host
        .expand_text_in_parse(
            &transaction,
            TextMacroRequest {
                context: context(4),
                source: MappedSource::identity("anchor"),
            },
        )
        .expect("anchored macro must run");

    assert_eq!(result.source.virtual_source(), "anchor generated");
    let generated = result
        .source
        .map_range(TextRange::new(6, 16))
        .expect("generated range must map")
        .primary_origin()
        .expect("generated range has an origin");
    assert_eq!(generated.original_range, TextRange::empty(0));
    assert_eq!(generated.kind, OriginKind::Anchored);
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn quota_errors_roll_back_text_macro_state() {
    let mut host = host(HostConfig {
        max_text_macro_generated_bytes: 4,
        ..HostConfig::default()
    });
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 5)
        .expect("parse must begin");
    let error = host
        .expand_text_in_parse(
            &transaction,
            TextMacroRequest {
                context: context(5),
                source: MappedSource::identity("alpha"),
            },
        )
        .expect_err("generated text quota must fail");

    assert!(matches!(
        error,
        HostError::TextMacroGeneratedBytesQuotaExceeded { limit: 4 }
    ));
    assert!(written_keys(&transaction).is_empty());
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn expansion_quota_rolls_back_all_prior_macro_state() {
    let mut host = host(HostConfig {
        max_text_macro_expansions: 1,
        ..HostConfig::default()
    });
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 7)
        .expect("parse must begin");
    let error = host
        .expand_text_in_parse(
            &transaction,
            TextMacroRequest {
                context: context(7),
                source: MappedSource::identity("alpha"),
            },
        )
        .expect_err("the second expansion must exceed the quota");

    assert!(matches!(
        error,
        HostError::TextMacroExpansionQuotaExceeded { limit: 1 }
    ));
    assert!(written_keys(&transaction).is_empty());
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn virtual_source_quota_rolls_back_macro_state() {
    let mut host = host(HostConfig {
        max_virtual_source_bytes: 10,
        ..HostConfig::default()
    });
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 8)
        .expect("parse must begin");
    let error = host
        .expand_text_in_parse(
            &transaction,
            TextMacroRequest {
                context: context(8),
                source: MappedSource::identity("anchor"),
            },
        )
        .expect_err("the expanded virtual source must exceed the quota");

    assert!(matches!(
        error,
        HostError::VirtualSourceQuotaExceeded { limit: 10 }
    ));
    assert!(written_keys(&transaction).is_empty());
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn traps_disable_the_component_and_discard_staged_state() {
    let mut host = host(HostConfig::default());
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 6)
        .expect("parse must begin");
    let result = host
        .expand_text_in_parse(
            &transaction,
            TextMacroRequest {
                context: context(6),
                source: MappedSource::identity("trap"),
            },
        )
        .expect("component traps are isolated failures");

    assert_eq!(result.source.virtual_source(), "trap");
    assert_eq!(result.failures.len(), 1);
    assert!(matches!(
        result.failures[0].error,
        HostError::Trap { .. } | HostError::FuelExhausted { .. } | HostError::Runtime { .. }
    ));
    assert!(
        host.components()
            .iter()
            .any(|component| component.component_id == COMPONENT_ID && component.disabled)
    );
    assert!(written_keys(&transaction).is_empty());
    transaction.cancel().expect("test parse may be cancelled");
}

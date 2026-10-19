use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use parser_wasm::{
    DocumentCancellationToken, DocumentParseError, DocumentParseRequest, DocumentParseStage,
    DocumentParserConfig, HostConfig, ParserHost,
};
use skript_parser::{RawNodeKind, SectionBodyNode, StructureBody, StructureDocumentNode};

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));
const VARIABLE_SCHEMA_ID: &str = "nlaocs.skript.variable";

fn host(fixture: impl AsRef<Path>) -> ParserHost {
    let catalog = Arc::new(
        ssg::load(fixture)
            .expect("fixture snapshot must load")
            .catalog()
            .clone(),
    );
    ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must initialize from the fixture profile")
}

fn modern_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn legacy_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/type-parser-versions/skript-2.6.4-mc-1.12.2")
}

fn current_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/type-parser-versions/skript-2.16.0")
}

fn request(revision: u64, source: &str) -> DocumentParseRequest {
    DocumentParseRequest::new(
        "file:///workspace",
        "file:///workspace/document.sk",
        revision,
        source.to_owned(),
    )
}

fn trigger_body(result: &parser_wasm::DocumentParseResult) -> &[SectionBodyNode] {
    result
        .syntax
        .roots
        .iter()
        .find_map(|root| match root {
            StructureDocumentNode::Structure(matches) => {
                matches
                    .selected
                    .as_ref()
                    .and_then(|structure| match &structure.body {
                        StructureBody::Trigger(body) => Some(body.as_slice()),
                        StructureBody::Entries(_) | StructureBody::None | StructureBody::Raw(_) => {
                            None
                        }
                    })
            }
            StructureDocumentNode::Trivia(_) | StructureDocumentNode::Unclaimed(_) => None,
        })
        .expect("document must contain a selected trigger Structure")
}

#[test]
fn parses_and_commits_a_document_with_public_expression_data() {
    let mut host = host(modern_fixture());
    let result = host
        .parse_document(
            request(1, "on load:\n    set {_value} to 1\n"),
            DocumentParserConfig::default(),
        )
        .expect("document pipeline must succeed");

    assert!(result.component_failures.is_empty());
    assert_eq!(result.syntax.roots.len(), 1);
    assert!(
        trigger_body(&result).iter().any(|node| matches!(
            node,
            SectionBodyNode::Effect(matches) if matches.selected.is_some()
        )),
        "the Effect body must be parsed"
    );
    let variables = result
        .ast
        .nodes
        .iter()
        .filter_map(|node| node.summary.as_ref())
        .flat_map(|summary| &summary.public_data)
        .filter(|entry| entry.schema_id == VARIABLE_SCHEMA_ID)
        .collect::<Vec<_>>();
    assert_eq!(variables.len(), 1);
    let variable: serde_json::Value =
        serde_json::from_str(&variables[0].json).expect("variable public data must be JSON");
    assert_eq!(variable["scope"], "local");
    assert_eq!(variable["name"][0]["text"], "value");
    assert_eq!(result.state.writes, 0);
}

#[test]
fn parses_bare_conditions_as_trigger_items() {
    let mut host = host(current_fixture());
    let result = host
        .parse_document(
            request(2, "on load:\n    1 is 1\n    send \"after\" to console\n"),
            DocumentParserConfig::default(),
        )
        .expect("a bare Condition must be accepted as a TriggerItem");
    let body = trigger_body(&result);

    assert!(matches!(
        body.first(),
        Some(SectionBodyNode::Condition { matches, .. }) if matches.selected.is_some()
    ));
    assert!(matches!(
        body.get(1),
        Some(SectionBodyNode::Effect(matches)) if matches.selected.is_some()
    ));
    assert!(result.syntax.diagnostics.is_empty());
}

#[test]
fn owned_metadata_links_conditional_section_siblings() {
    let mut host = host(current_fixture());
    let result = host
        .parse_document(
            request(
                2,
                "on load:\n    if 1 is 1:\n        send \"yes\" to console\n    # comments do not break Skript's conditional chain\n    else if 2 is 2:\n        send \"maybe\" to console\n    else:\n        send \"no\" to console\n",
            ),
            DocumentParserConfig::default(),
        )
        .expect("conditional document must parse");
    let conditionals = trigger_body(&result)
        .iter()
        .filter_map(|node| match node {
            SectionBodyNode::Section(matches) => matches.selected.as_ref(),
            _ => None,
        })
        .collect::<Vec<_>>();

    let kinds = conditionals
        .iter()
        .map(|conditional| {
            conditional
                .metadata
                .get("nlaocs.core-library/conditional-kind")
                .map(String::as_str)
        })
        .collect::<Vec<_>>();

    assert_eq!(kinds, [Some("if"), Some("else-if"), Some("else")]);
    assert!(
        conditionals[2].body.iter().any(|node| matches!(
            node,
            SectionBodyNode::Effect(matches) if matches.selected.is_some()
        )),
        "the else body must remain attached to the claimed branch"
    );
    assert!(result.syntax.diagnostics.is_empty());
}

#[test]
fn unclaimed_items_do_not_interrupt_conditional_section_siblings() {
    let mut host = host(current_fixture());
    let result = host
        .parse_document(
            request(
                3,
                "on load:\n    if 1 is 1:\n        send \"yes\" to console\n    this effect does not exist\n    else:\n        send \"no\" to console\n",
            ),
            DocumentParserConfig::default(),
        )
        .expect("syntax recovery must preserve the conditional chain");
    let body = trigger_body(&result);

    assert_eq!(
        body.iter()
            .filter(|node| matches!(
                node,
                SectionBodyNode::Section(matches) if matches.selected.is_some()
            ))
            .count(),
        2,
        "the invalid Effect is not a TriggerItem and must not orphan else"
    );
    assert!(body.iter().any(|node| matches!(
        node,
        SectionBodyNode::Effect(matches)
            if matches.selected.is_none() && matches.unknown.is_some()
    )));
}

#[test]
fn selected_effect_interrupts_conditional_section_siblings() {
    let mut host = host(current_fixture());
    let result = host
        .parse_document(
            request(
                4,
                "on load:\n    if 1 is 1:\n        send \"yes\" to console\n    send \"separator\" to console\n    else:\n        send \"no\" to console\n",
            ),
            DocumentParserConfig::default(),
        )
        .expect("an orphan else must remain recoverable");

    assert!(trigger_body(&result).iter().any(|node| matches!(
        node,
        SectionBodyNode::Section(matches)
            if matches.selected.is_none()
                && matches.unknown.as_ref().is_some_and(|unknown| unknown.source == "else")
    )));
}

#[test]
fn parses_tree_macro_generated_effect_text() {
    let mut host = host(modern_fixture());
    let result = host
        .parse_document(
            request(
                2,
                "options:\n    message: send 1 to console\non load:\n    {@message}\n",
            ),
            DocumentParserConfig::default(),
        )
        .expect("options expansion and syntax parsing must succeed");

    assert!(
        result
            .tree_macro_calls
            .iter()
            .any(|call| call.accepted && call.expansion.is_some()),
        "CoreLibrary options expansion must run"
    );
    let event = result
        .syntax
        .roots
        .iter()
        .filter_map(|root| match root {
            StructureDocumentNode::Structure(matches) => matches.selected.as_ref(),
            StructureDocumentNode::Trivia(_) | StructureDocumentNode::Unclaimed(_) => None,
        })
        .find(|structure| matches!(structure.body, StructureBody::Trigger(_)))
        .expect("expanded document must contain an Event Structure");
    let StructureBody::Trigger(body) = &event.body else {
        unreachable!();
    };
    let effect = body
        .iter()
        .find_map(|node| match node {
            SectionBodyNode::Effect(matches) => matches.selected.as_ref(),
            _ => None,
        })
        .expect("generated `send 1 to console` must parse as an Effect");
    let range = effect.matched.matched.span.mapped.virtual_range;
    assert_eq!(
        range.slice(result.source.virtual_source()),
        Some("send 1 to console")
    );
    assert!(range.start >= result.source.original().len());
    let mapped = result
        .source
        .map_range(range)
        .expect("generated Effect span must map to the original document");
    let origin = mapped
        .primary_origin()
        .expect("generated Effect span must retain its call site");
    assert_eq!(
        origin.original_range.slice(result.source.original()),
        Some("    {@message}\n")
    );
    let expansion = origin
        .expansion
        .expect("generated Effect span must identify its Tree expansion");
    let backtrace = result
        .source
        .expansion_backtrace(expansion)
        .expect("generated Effect span must expose an expansion backtrace");
    assert_eq!(backtrace[0].component.as_str(), "nlaocs.core-library");
}

#[test]
fn maps_tree_macro_generated_failures_to_the_call_site() {
    let mut host = host(modern_fixture());
    let result = host
        .parse_document(
            request(
                3,
                "options:\n    broken: this effect does not exist\non load:\n    {@broken}\n",
            ),
            DocumentParserConfig::default(),
        )
        .expect("an unknown generated Effect must remain recoverable");
    let unknown = trigger_body(&result)
        .iter()
        .find_map(|node| match node {
            SectionBodyNode::Effect(matches) => matches.unknown.as_ref(),
            _ => None,
        })
        .expect("generated invalid text must remain as an unknown Effect");
    let diagnostic = unknown
        .failures
        .primary()
        .map(|failure| &failure.matched.trace.failure.span.mapped)
        .or_else(|| {
            unknown
                .failures
                .fallback
                .as_ref()
                .map(|failure| &failure.failure.span.mapped)
        })
        .expect("unknown generated Effect must retain a diagnostic span");
    let origin = diagnostic
        .primary_origin()
        .expect("generated diagnostic must retain its call site");
    assert_eq!(
        origin.original_range.slice(result.source.original()),
        Some("    {@broken}\n")
    );
    let expansion = origin
        .expansion
        .expect("generated diagnostic must identify its Tree expansion");
    assert!(
        result
            .source
            .expansion_backtrace(expansion)
            .is_some_and(|trace| !trace.is_empty())
    );
}

#[test]
fn preserves_unknown_nodes_and_continues_parsing_later_lines() {
    let mut host = host(modern_fixture());
    let result = host
        .parse_document(
            request(
                4,
                "on load:\n    this effect does not exist\n    send 1 to console\n",
            ),
            DocumentParserConfig::default(),
        )
        .expect("syntax errors must produce a partial document");
    let body = trigger_body(&result);

    assert!(body.iter().any(|node| matches!(
        node,
        SectionBodyNode::Effect(matches) if matches.selected.is_none() && matches.unknown.is_some()
    )));
    assert!(body.iter().any(|node| matches!(
        node,
        SectionBodyNode::Effect(matches) if matches.selected.is_some()
    )));
}

#[test]
fn derives_multiline_comment_rules_from_the_snapshot_version() {
    let source = "###\nsend 1\n###\n";
    let mut modern = host(modern_fixture());
    let modern = modern
        .parse_document(request(5, source), DocumentParserConfig::default())
        .expect("modern document must parse");
    let mut legacy = host(legacy_fixture());
    let legacy = legacy
        .parse_document(request(5, source), DocumentParserConfig::default())
        .expect("legacy document must parse");

    assert!(
        modern
            .raw_tree
            .nodes
            .iter()
            .all(|node| node.kind != RawNodeKind::Simple)
    );
    assert!(
        legacy
            .raw_tree
            .nodes
            .iter()
            .any(|node| node.kind == RawNodeKind::Simple && node.text == "send 1")
    );
}

#[test]
fn legacy_and_modern_snapshots_share_the_document_parser_api() {
    const SOURCE: &str = "on dummy fixture event with priority high:\n    dummy effect registered through wrapper\nfunction fixture():\n    dummy effect registered through wrapper\ncommand /fixture:\n    trigger:\n        dummy effect registered through wrapper\n";

    for (fixture, legacy) in [(legacy_fixture(), true), (current_fixture(), false)] {
        let mut host = host(fixture);
        let transaction = host
            .begin_parse("file:///workspace", "file:///workspace/document.sk", 10)
            .expect("dynamic syntax snapshot must begin");
        let dynamic = host
            .dynamic_syntax_snapshot(&transaction)
            .expect("dynamic syntax registrations must freeze");
        let legacy_registrations = dynamic
            .definitions
            .keys()
            .filter(|id| {
                id.component_id == "nlaocs.core-library"
                    && id.local_id.starts_with("legacy-struct-")
            })
            .count();
        assert_eq!(legacy_registrations, if legacy { 3 } else { 0 });
        transaction.cancel().expect("inspection may be cancelled");

        let result = host
            .parse_document(request(11, SOURCE), DocumentParserConfig::default())
            .expect("legacy and modern documents must use the same parser API");
        assert!(result.component_failures.is_empty(), "{result:#?}");
        assert_eq!(result.ast.roots.len(), 3, "{result:#?}");
        assert_eq!(result.functions.registrations().len(), 1, "{result:#?}");

        let selected = result
            .syntax
            .roots
            .iter()
            .filter_map(|root| match root {
                StructureDocumentNode::Structure(matches) => matches.selected.as_ref(),
                StructureDocumentNode::Trivia(_) | StructureDocumentNode::Unclaimed(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(selected.len(), 3, "{result:#?}");

        for semantic_mode in ["event-structure", "function-structure", "command-structure"] {
            let structure = selected
                .iter()
                .find(|structure| {
                    structure
                        .metadata
                        .get("nlaocs.core-library/semantic-mode")
                        .is_some_and(|mode| mode == semantic_mode)
                })
                .unwrap_or_else(|| panic!("missing {semantic_mode}: {result:#?}"));
            let is_legacy_registration = structure
                .matched
                .registration_id
                .starts_with("dynamic:nlaocs.core-library/legacy-struct-");
            assert_eq!(is_legacy_registration, legacy, "{structure:#?}");
        }

        let event = selected
            .iter()
            .find(|structure| {
                structure
                    .metadata
                    .get("nlaocs.core-library/semantic-mode")
                    .is_some_and(|mode| mode == "event-structure")
            })
            .expect("event Structure must be selected");
        assert_eq!(
            event
                .metadata
                .get("nlaocs.core-library/event-priority")
                .map(String::as_str),
            Some("high")
        );

        let command = selected
            .iter()
            .find(|structure| {
                structure
                    .metadata
                    .get("nlaocs.core-library/semantic-mode")
                    .is_some_and(|mode| mode == "command-structure")
            })
            .expect("command Structure must be selected");
        assert!(matches!(command.body, StructureBody::Entries(_)));
    }
}

#[test]
fn cancellation_before_start_leaves_the_host_reusable() {
    let mut host = host(modern_fixture());
    let cancellation = DocumentCancellationToken::new();
    cancellation.cancel();
    let error = host
        .parse_document(
            request(6, "on load:\n    send 1\n"),
            DocumentParserConfig {
                cancellation,
                ..DocumentParserConfig::default()
            },
        )
        .expect_err("cancelled document must not parse");
    assert!(matches!(
        error,
        DocumentParseError::Cancelled {
            stage: DocumentParseStage::Start,
            ..
        }
    ));

    host.parse_document(
        request(7, "on load:\n    send 1\n"),
        DocumentParserConfig::default(),
    )
    .expect("a cancelled request must not poison the next revision");
}

#[test]
fn cancellation_after_prepared_core_header_rolls_back_and_allows_same_revision_reentry() {
    use parser_wasm::state::{NamespaceVisibility, StateScope};
    use skript_parser::StructurePreparationEvent;

    const REVISION: u64 = 60;
    const SOURCE: &str = "command /cancel-lifecycle:\n    trigger:\n        send 1 to console\n";
    let mut host = host(modern_fixture());
    let cancellation = DocumentCancellationToken::new();
    let mut saw_prepared_command = false;
    let mut observer = |event: StructurePreparationEvent<'_>| {
        if let StructurePreparationEvent::Headers {
            source,
            tree,
            headers,
        } = event
        {
            assert_eq!(source.original(), SOURCE);
            assert!(!tree.nodes.is_empty());
            saw_prepared_command = headers.iter().any(|header| {
                header
                    .candidate
                    .metadata
                    .get("nlaocs.core-library/semantic-mode")
                    .is_some_and(|mode| mode == "command-structure")
            });
            assert!(
                saw_prepared_command,
                "Core must accept and initialize the command header before cancellation"
            );
            cancellation.cancel();
        }
    };
    let error = host
        .parse_document_with_preparation(
            request(REVISION, SOURCE),
            DocumentParserConfig {
                cancellation: cancellation.clone(),
                ..DocumentParserConfig::default()
            },
            &mut observer,
        )
        .expect_err("cancellation after provisional headers must prevent adoption and commit");
    assert!(
        saw_prepared_command,
        "the cancellation must occur after real native/Core work"
    );
    assert!(matches!(error, DocumentParseError::Cancelled {
        document_id,
        document_revision: REVISION,
        stage: DocumentParseStage::Syntax,
    } if document_id == "file:///workspace/document.sk"));
    assert!(
        host.components()
            .iter()
            .all(|component| !component.disabled)
    );

    // Core has already staged this command name in its private Parse scope.
    // A new same-revision transaction must see no leaked parse or document value.
    let inspect = host
        .begin_parse(
            "file:///workspace",
            "file:///workspace/document.sk",
            REVISION,
        )
        .unwrap();
    let mut invocation = inspect.begin_invocation("nlaocs.core-library").unwrap();
    for scope in [StateScope::Parse, StateScope::Document] {
        assert!(
            invocation
                .get(
                    scope,
                    NamespaceVisibility::Private,
                    "commands",
                    "cancel-lifecycle"
                )
                .unwrap()
                .is_none()
        );
    }
    invocation.rollback();
    inspect.cancel().unwrap();

    for revision in [REVISION, REVISION + 1] {
        let result = host
            .parse_document(request(revision, SOURCE), DocumentParserConfig::default())
            .expect("cancelled work must not poison the same or following revision");
        assert!(result.component_failures.is_empty());
        assert!(
            result.syntax.diagnostics.is_empty(),
            "{:#?}",
            result.syntax.diagnostics
        );
        assert_eq!(result.ast.roots.len(), 1);
        assert_eq!(
            result.state.writes, 0,
            "Core command state must remain parse-private"
        );
        assert!(
            result.state.read_write_set.writes.iter().any(|record| {
                record.scope == StateScope::Parse
                    && record.namespace == "commands"
                    && record.key == "cancel-lifecycle"
            }),
            "the successful retry must actually initialize the same command name again"
        );
        assert!(
            host.components()
                .iter()
                .all(|component| !component.disabled)
        );
    }
}

#[test]
fn stale_document_revision_cannot_commit_or_poison_a_newer_parse() {
    let mut host = host(modern_fixture());
    let newer = host
        .begin_parse("file:///workspace", "file:///workspace/document.sk", 8)
        .expect("newer revision must begin");

    let error = host
        .parse_document(
            request(7, "on load:\n    send 1\n"),
            DocumentParserConfig::default(),
        )
        .expect_err("stale revision must fail before commit");
    assert!(matches!(
        error,
        DocumentParseError::StaleRevision {
            actual: 7,
            latest: 8,
            ..
        }
    ));

    newer
        .cancel()
        .expect("unused newer transaction may be cancelled");
    host.parse_document(
        request(9, "on load:\n    send 1\n"),
        DocumentParserConfig::default(),
    )
    .expect("a stale revision must not poison a later revision");
}

#[test]
fn extension_fault_preserves_an_independent_later_line_and_its_cause() {
    let mut host = host(modern_fixture());
    host.load_addon(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../artifacts/expression-data-addon-a.wasm"
    )))
    .unwrap();
    let result = host
        .parse_document(
            request(
                50,
                "on load:\n    set {invalid-json::*} to 1\n    send 1 to console\n",
            ),
            DocumentParserConfig::default(),
        )
        .expect("recover execution faults at the affected raw node");
    assert_eq!(
        result.execution_status,
        parser_wasm::host::DocumentExecutionStatus::Partial
    );
    assert!(
        trigger_body(&result)
            .iter()
            .any(|node| matches!(node, SectionBodyNode::Unclaimed(_)))
    );
    assert!(trigger_body(&result).iter().any(
        |node| matches!(node, SectionBodyNode::Effect(matches) if matches.selected.is_some())
    ));
    assert!(
        result
            .component_failures
            .iter()
            .any(|failure| failure.component_id == "test.expression-data-a"
                && failure.subscription_id == "expression-data.a.invalid")
    );
    assert!(
        result
            .fault_regions
            .iter()
            .any(|region| region.fault.code == "parser.extension-invalid-output")
    );
}

#[test]
fn text_macro_trap_returns_known_source_without_certifying_standard_syntax() {
    let mut host = host(modern_fixture());
    host.load_addon(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../artifacts/text-macro-addon.wasm"
    )))
    .unwrap();
    let source = "on load:\n    # trap\n    send 1 to console\n";
    let result = host
        .parse_document(request(51, source), DocumentParserConfig::default())
        .expect("source is available as a partial result");
    assert_eq!(result.source.virtual_source(), source);
    assert_eq!(
        result.execution_status,
        parser_wasm::host::DocumentExecutionStatus::Partial
    );
    assert!(result.syntax.roots.iter().all(|node| !matches!(node, StructureDocumentNode::Structure(matches) if matches.selected.is_some())));
    assert!(
        result
            .fault_regions
            .iter()
            .any(|region| region.fault.code == "parser.extension-trap")
    );
    let later = host
        .parse_document(
            request(52, "on load:\n    send 1 to console\n"),
            DocumentParserConfig::default(),
        )
        .expect("disabled component remains distinguishable from unsupported syntax");
    assert_eq!(
        later.execution_status,
        parser_wasm::host::DocumentExecutionStatus::Partial
    );
    assert!(
        later
            .fault_regions
            .iter()
            .any(|region| region.fault.code == "parser.extension-unavailable")
    );
}

#[test]
fn extension_fault_in_one_root_preserves_an_independent_later_root() {
    let mut host = host(modern_fixture());
    host.load_addon(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../artifacts/expression-data-addon-a.wasm"
    )))
    .unwrap();
    let result = host
        .parse_document(
            request(
                53,
                "on load:\n    set {invalid-json::*} to 1\non load:\n    send 1 to console\n",
            ),
            DocumentParserConfig::default(),
        )
        .unwrap();
    assert_eq!(
        result.execution_status,
        parser_wasm::host::DocumentExecutionStatus::Partial
    );
    let bodies: Vec<_> = result
        .syntax
        .roots
        .iter()
        .filter_map(|root| match root {
            StructureDocumentNode::Structure(matches) => {
                matches
                    .selected
                    .as_ref()
                    .and_then(|selected| match &selected.body {
                        StructureBody::Trigger(body) => Some(body),
                        _ => None,
                    })
            }
            _ => None,
        })
        .collect();
    assert_eq!(bodies.len(), 2);
    assert!(
        bodies[0]
            .iter()
            .any(|node| matches!(node, SectionBodyNode::Unclaimed(_)))
    );
    assert!(bodies[1].iter().any(
        |node| matches!(node, SectionBodyNode::Effect(matches) if matches.selected.is_some())
    ));
    assert!(
        result
            .component_failures
            .iter()
            .any(|failure| failure.component_id == "test.expression-data-a"
                && failure.subscription_id == "expression-data.a.invalid")
    );
}

#[test]
fn editor_ast_recovers_entry_failures_without_adopting_rejected_structures() {
    let mut host = host(modern_fixture());
    let mut result=host.parse_document(request(90,"on load:\n    send 0 to console\ncommand /test:\n    permission: one\n    permission: two\n    trigger:\n        send 1 to console\n"),DocumentParserConfig::default()).unwrap();
    let canonical = format!("{:?}", result.ast);
    assert!(
        !result
            .ast
            .nodes
            .iter()
            .any(|node| node.text == "send 1 to console")
    );
    let view = result.editor_ast();
    assert!(
        view.nodes
            .iter()
            .any(|node| node.text == "send 1 to console")
    );
    assert!(!view.nodes.iter().any(|node| node.text == "command /test"));
    let ids = view
        .nodes
        .iter()
        .map(|node| node.id)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), view.nodes.len());
    assert!(view.roots.iter().all(|id| ids.contains(id)));
    assert!(
        view.nodes
            .iter()
            .all(|node| node.children.iter().all(|id| ids.contains(id)))
    );
    assert_eq!(canonical, format!("{:?}", result.ast));
    let StructureDocumentNode::Structure(command) = &mut result.syntax.roots[1] else {
        panic!()
    };
    let candidate = command.unknown.as_mut().unwrap().partial.as_mut().unwrap();
    assert!(candidate.entry_validation_failed);
    // A semantic exit rejection carries a partial tree but never this eligibility bit.
    candidate.entry_validation_failed = false;
    assert_eq!(canonical, format!("{:?}", result.editor_ast()));
}

#[test]
fn legacy_command_trigger_contract_remains_executable() {
    let mut host = host(legacy_fixture());
    let result = host
        .parse_document(
            request(
                91,
                "command /test:\n    trigger:\n        send 1 to console\n",
            ),
            DocumentParserConfig::default(),
        )
        .unwrap();
    assert!(
        result.component_failures.is_empty(),
        "{:?}",
        result.component_failures
    );
    assert!(result.fault_regions.is_empty());
    assert!(
        result
            .ast
            .nodes
            .iter()
            .any(|node| node.text == "send 1 to console"),
        "{:#?}",
        result.syntax
    );
}

#[test]
fn command_and_event_arithmetic_keep_the_same_body_diagnostics() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/type-parser-versions/skript-2.16.2-java25");
    let mut host = host(fixture);
    let sources = [
        "on load:\n    if 1 is 1:\n        send 1 + 2 * 3 to console\n",
        "command /arithmetic:\n    trigger:\n        if 1 is 1:\n            send 1 + 2 * 3 to console\n",
    ];
    let mut signatures = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        let result = host
            .parse_document(
                request(100 + index as u64, source),
                DocumentParserConfig::default(),
            )
            .unwrap();
        assert!(result.component_failures.is_empty());
        let diagnostics = result
            .syntax
            .diagnostics
            .iter()
            .map(|d| {
                (
                    d.kind,
                    d.message.clone(),
                    d.span
                        .mapped
                        .virtual_range
                        .slice(result.source.virtual_source())
                        .unwrap()
                        .to_owned(),
                )
            })
            .collect::<Vec<_>>();
        println!("arithmetic-body,{index},{diagnostics:?}");
        signatures.push(diagnostics);
    }
    assert_eq!(
        signatures[0], signatures[1],
        "Command must use the same native body path as Event"
    );
}

#[test]
fn native_function_statements_share_executable_bodies_and_preserve_published_proof() {
    // A Core-only positive fixture must not acquire overlapping addon syntax.
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/type-parser-versions/skript-2.15.4-type-runtime");
    let snapshot = ssg::load(&fixture).expect("genuine Core snapshot must load");
    let return_registration = snapshot
        .catalog()
        .effects()
        .find(|effect| effect.common.element_class.as_str() == "ch.njol.skript.effects.EffReturn")
        .expect("genuine snapshot has the standard return Effect")
        .common
        .registration_id
        .as_str()
        .to_owned();
    let comparison_registration = snapshot
        .catalog()
        .conditions()
        .find(|condition| {
            condition.common.element_class.as_str() == "ch.njol.skript.conditions.CondCompare"
        })
        .expect("genuine snapshot has the standard comparison Condition")
        .common
        .registration_id
        .as_str()
        .to_owned();
    let mut host = host(fixture);
    let text = "function procedure(value: number):\n    send {_value} to console\nfunction computed(value: number = 1) :: number:\n    return {_value}\nfunction wrapper():\n    procedure(1)\non load:\n    procedure(value: 2)\n    computed()\n    if 1 is 1:\n        procedure(3)\n    procedure(5) if 1 is 1\ncommand /fixture:\n    trigger:\n        procedure(4)\n";
    let result = host
        .parse_document(request(70, text), DocumentParserConfig::default())
        .unwrap();
    assert!(
        result.component_failures.is_empty(),
        "{:#?}",
        result.component_failures
    );
    assert!(
        result.syntax.diagnostics.is_empty(),
        "{:#?}",
        result.syntax.diagnostics
    );
    assert!(
        result
            .ast
            .nodes
            .iter()
            .any(|node| node.text == "return {_value}"
                && node
                    .summary
                    .as_ref()
                    .and_then(|summary| summary.registration_id.as_deref())
                    == Some(return_registration.as_str())),
        "the value-returning function retains the genuine standard return Effect"
    );
    assert_eq!(
        result
            .ast
            .nodes
            .iter()
            .filter(|node| node.text == "1 is 1"
                && node
                    .summary
                    .as_ref()
                    .and_then(|summary| summary.registration_id.as_deref())
                    == Some(comparison_registration.as_str()))
            .count(),
        2,
        "the nested section and routed conditional Effect both retain real comparison nodes"
    );
    let calls = result
        .ast
        .nodes
        .iter()
        .filter(|node| node.function.is_some())
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 6, "{:#?}", result.ast);
    let witnesses = result.semantic_witnesses();
    assert!(
        result
            .ast
            .nodes
            .iter()
            .all(|node| witnesses.native_valid(node)),
        "new Function nodes must not invalidate the native forest proof"
    );
    for node in calls {
        let call = node.function.as_ref().unwrap();
        assert_eq!(node.kind, parser_wasm::host::SyntaxKind::Function);
        assert_eq!(
            &text[node.span.virtual_range.start as usize..node.span.virtual_range.end as usize],
            node.text
        );
        assert!(call.definition_id.starts_with("function:"));
        assert!(!call.registration_id.is_empty());
        assert!(
            node.metadata.iter().any(
                |entry| entry.owner_component_id.is_none() && entry.key == "function.signature"
            )
        );
        assert!(
            node.metadata
                .iter()
                .any(|entry| entry.owner_component_id.is_none()
                    && entry.key == "function.document-id"
                    && entry.value == "file:///workspace/document.sk")
        );
        assert!(
            node.metadata
                .iter()
                .any(|entry| entry.owner_component_id.is_none()
                    && entry.key == "function.revision"
                    && entry.value == "70")
        );
        if call.name == "procedure" {
            assert!(node.summary.as_ref().unwrap().return_type.is_none());
            assert!(node.summary.as_ref().unwrap().multiplicity.is_none());
            assert_eq!(node.children.len(), 1);
            let child = result
                .ast
                .nodes
                .iter()
                .find(|child| child.id == node.children[0])
                .unwrap();
            assert!(
                child.span.virtual_range.start >= node.span.virtual_range.start
                    && child.span.virtual_range.end <= node.span.virtual_range.end
            );
        } else {
            assert_eq!(call.name, "computed");
            assert!(node.summary.as_ref().unwrap().return_type.is_some());
            assert!(call.arguments[0].omitted);
        }
    }
}

#[test]
fn rejected_function_statement_recovers_following_statements_and_does_not_claim_partial_calls() {
    let mut host = host(modern_fixture());
    for (revision, bad) in [
        (71, "procedure()"),
        (72, "procedure(1) trailing"),
        (73, "procedure("),
    ] {
        let text = format!(
            "function procedure(value: number):\n    send {{_value}} to console\non load:\n    {bad}\n    send log(8) to console\n"
        );
        let result = host
            .parse_document(request(revision, &text), DocumentParserConfig::default())
            .unwrap();
        assert!(
            result.component_failures.is_empty(),
            "{bad}: {:#?}",
            result.component_failures
        );
        assert!(
            !result.ast.nodes.iter().any(|node| node
                .function
                .as_ref()
                .is_some_and(|call| call.name == "procedure")),
            "invalid/partial call was published: {bad}"
        );
        assert!(
            result.ast.nodes.iter().any(|node| node
                .function
                .as_ref()
                .is_some_and(|call| call.name == "log")),
            "following ordinary Effect containing a function expression must recover: {bad}"
        );
        assert!(
            !result.syntax.diagnostics.is_empty() || !result.raw_tree.diagnostics.is_empty(),
            "{bad} must retain a diagnostic"
        );
    }
}

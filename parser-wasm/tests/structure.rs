use parser_wasm::host::{HostConfig, InvocationContext, ParserHost, RuntimeProfile};
use skript_parser::{
    EffectMatches, ExpressionParseContext, FailureTrace, MappedSource, ParsedCaptureValue,
    PatternFailureReason, RawTreeOptions, SectionBodyNode, StructureBody, StructureDiagnosticKind,
    StructureDocumentNode, StructureEntryValue, StructureParseRequest, StructureParserConfig,
    parse_raw_tree,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use syntaxes::{Catalog, CatalogParts, ClassName, DefinitionId, Pattern, RegistrationId, Syntax};

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));
const EFFECT_ADDON: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/effect-addon.wasm"
));

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn catalog() -> Arc<Catalog> {
    Arc::new(ssg::load(fixture()).unwrap().catalog().clone())
}

fn structure_enter_rejection_fallback_catalog() -> Arc<Catalog> {
    let snapshot = ssg::load(fixture()).unwrap();
    let source = snapshot.catalog();
    let source_view = source.source().cloned().expect("SSG source view");
    let mut syntaxes = source.syntaxes().to_vec();
    let mut fallback = source
        .structures()
        .find(|structure| {
            structure.common.element_class.as_str() == "ch.njol.skript.structures.StructVariables"
        })
        .expect("StructVariables fixture")
        .clone();
    let pattern_source = "variables";
    fallback.common.definition_id = DefinitionId("structure:test:variables-fallback".to_owned());
    fallback.common.registration_id =
        RegistrationId("structure:test:variables-fallback:0".to_owned());
    fallback.common.registration_order = usize::MAX;
    fallback.common.element_class = ClassName("test.GenericVariablesStructure".to_owned());
    fallback.common.patterns = vec![Pattern {
        source: pattern_source.to_owned(),
        parsed: syntax_pattern_parser::syntax::parse(pattern_source, source.plural_rules())
            .expect("fallback Structure pattern must parse"),
    }];
    fallback.entry_validator = None;
    syntaxes.push(Syntax::Structure(fallback));
    Arc::new(
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
        .with_unchecked_source(source_view),
    )
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
    let (transaction, result) = parse_with_transaction(host, revision, input);
    transaction.cancel().unwrap();
    result
}

fn parse_with_transaction(
    host: &mut ParserHost,
    revision: u64,
    input: &str,
) -> (
    parser_wasm::state::ParseTransaction,
    parser_wasm::WasmStructureParseResult,
) {
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
    (transaction, result)
}

fn selected(result: &parser_wasm::WasmStructureParseResult) -> &skript_parser::StructureCandidate {
    selected_at(result, 0)
}

fn selected_at(
    result: &parser_wasm::WasmStructureParseResult,
    index: usize,
) -> &skript_parser::StructureCandidate {
    let StructureDocumentNode::Structure(matches) = &result.document.roots[index] else {
        panic!("top-level node {index} must use the Structure pipeline");
    };
    matches.selected.as_ref().unwrap_or_else(|| {
        panic!(
            "Structure must match: {matches:#?}\neffects: {:#?}\ncalls: {:#?}\nfailures: {:#?}",
            result.effects, result.calls, result.failures
        )
    })
}

fn body_effect(result: &parser_wasm::WasmStructureParseResult) -> &EffectMatches {
    let candidate = selected(result);
    let StructureBody::Trigger(body) = &candidate.body else {
        panic!(
            "StructEvent must parse its body as a trigger: {candidate:#?}\ncalls: {:#?}\neffects: {:#?}",
            result.calls, result.effects
        );
    };
    let Some(SectionBodyNode::Effect(effect)) = body.first() else {
        panic!("StructEvent body must contain one Effect node: {body:#?}");
    };
    effect
}

fn event_restriction_reason(trace: &FailureTrace) -> Option<(&[String], &[String])> {
    if let Some(PatternFailureReason::EventRestricted { supported, current }) = trace
        .failure
        .reasons
        .iter()
        .find(|reason| matches!(reason, PatternFailureReason::EventRestricted { .. }))
    {
        return Some((supported.as_slice(), current.as_slice()));
    }
    trace.cause.as_deref().and_then(event_restriction_reason)
}

fn effect_event_restriction_reason(effect: &EffectMatches) -> Option<(&[String], &[String])> {
    let unknown = effect.unknown.as_ref()?;
    unknown
        .failures
        .candidates
        .iter()
        .find_map(|candidate| {
            event_restriction_reason(&candidate.matched.trace).or_else(|| {
                candidate
                    .matched
                    .related
                    .iter()
                    .find_map(event_restriction_reason)
            })
        })
        .or_else(|| {
            unknown
                .failures
                .fallback
                .as_ref()
                .and_then(event_restriction_reason)
        })
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
fn effect_semantics_run_inside_structure_trigger_bodies() {
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
        2,
        "on dummy fixture event:\n    exit 2 sections\n",
    );
    assert!(
        result.calls.iter().any(|call| {
            call.component_id == "nlaocs.core-library"
                && call.subscription_id == "core.structure-semantics"
        }),
        "Structure hook did not run; calls: {:#?}; failures: {:#?}; effects: {:#?}",
        result.calls,
        result.failures,
        result.effects
    );
    let effect = body_effect(&result);

    assert!(
        effect.selected.is_none(),
        "EffExit must reject an unavailable Section depth: {effect:#?}"
    );
    assert!(effect.unknown.is_some());
    let failure = format!("{:#?}", effect.unknown);
    assert!(
        failure.contains("cannot exit 2 sections; only 0 are present"),
        "EffExit semantic rejection must survive candidate rollback: {failure}"
    );
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
    let function_result = parse(
        &mut host,
        2,
        "function fixture():\n    dummy effect registered through wrapper\n",
    );
    assert_eq!(
        function_result.functions.registrations().len(),
        1,
        "{function_result:#?}"
    );
    assert_eq!(
        function_result.functions.registrations()[0]
            .declaration
            .name,
        "fixture"
    );
    let function = selected(&function_result);
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
fn function_and_command_wait_for_every_default_parse_result() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            ..HostConfig::default()
        },
    )
    .unwrap();
    let function_result = parse(
        &mut host,
        4,
        "function fixture(first: number = 1, second: number = 2):\n    dummy effect registered through wrapper\n",
    );
    let function = selected(&function_result);
    assert!(function_result.failures.is_empty(), "{function_result:#?}");
    assert_eq!(function_result.functions.registrations().len(), 1);
    assert_eq!(
        function_result.functions.registrations()[0]
            .declaration
            .parameters
            .len(),
        2
    );
    assert_eq!(
        function
            .metadata
            .get("nlaocs.core-library/semantic-mode")
            .map(String::as_str),
        Some("function-structure")
    );

    let command_result = parse(
        &mut host,
        5,
        "command /fixture <first: number = 1> <second: number = 2>:\n    trigger:\n        dummy effect registered through wrapper\n",
    );
    let command = selected(&command_result);
    assert!(command_result.failures.is_empty(), "{command_result:#?}");
    assert_eq!(
        command
            .metadata
            .get("nlaocs.core-library/command.argument-count")
            .map(String::as_str),
        Some("2")
    );
}

#[test]
fn command_entries_report_native_cross_entry_warnings() {
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
        "command /fixture:\n    permission message: \"denied\"\n    cooldown message: \"wait\"\n    cooldown storage: {cooldown}\n    trigger:\n        dummy effect registered through wrapper\n",
    );
    selected(&result);
    let codes = result
        .effects
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<Vec<_>>();
    assert!(
        codes
            .iter()
            .any(|code| code.ends_with("permission-message-without-permission")),
        "{:#?}",
        result.effects.diagnostics
    );
    assert!(
        codes
            .iter()
            .any(|code| code.ends_with("cooldown-message-without-cooldown")),
        "{:#?}",
        result.effects.diagnostics
    );
    assert!(
        codes
            .iter()
            .any(|code| code.ends_with("cooldown-storage-without-cooldown")),
        "{:#?}",
        result.effects.diagnostics
    );
}

#[test]
fn variables_structure_parses_and_checks_literal_defaults() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            ..HostConfig::default()
        },
    )
    .unwrap();
    let valid = parse(
        &mut host,
        5,
        "variables:\n    {message} = \"hello\"\n    {count} = 42\n",
    );
    let candidate = selected(&valid);
    assert_eq!(
        candidate
            .metadata
            .get("nlaocs.core-library/variables-values")
            .map(String::as_str),
        Some("resolved"),
        "{valid:#?}"
    );
    assert!(
        valid.effects.diagnostics.iter().all(|diagnostic| {
            !diagnostic.code.ends_with("invalid-value")
                && !diagnostic.code.ends_with("value-not-serializable")
        }),
        "{:#?}",
        valid.effects.diagnostics
    );

    let invalid = parse(
        &mut host,
        6,
        "variables:\n    {broken} = this is not a registered literal\n",
    );
    selected(&invalid);
    assert!(
        invalid
            .effects
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.ends_with("invalid-value")),
        "{:#?}",
        invalid.effects.diagnostics
    );

    let placeholders = parse(
        &mut host,
        7,
        "variables:\n    {valid::%number%} = 1\n    {invalid::%double%} = 2\n",
    );
    selected(&placeholders);
    assert!(
        placeholders.effects.diagnostics.iter().any(|diagnostic| {
            diagnostic.code.ends_with("unknown-placeholder-type")
                && diagnostic.message.contains("double")
        }),
        "{:#?}",
        placeholders.effects.diagnostics
    );
    assert!(
        placeholders
            .effects
            .diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.message.contains("number`")),
        "{:#?}",
        placeholders.effects.diagnostics
    );
}

#[test]
fn raw_builtin_structures_publish_metadata_without_duplicate_keys() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            ..HostConfig::default()
        },
    )
    .unwrap();
    for (revision, source) in [
        (7, "options:\n    greeting: hello\n"),
        (8, "aliases:\n    building stone = stone\n"),
        (9, "auto reload\n"),
    ] {
        let result = parse(&mut host, revision, source);
        selected(&result);
        assert!(result.failures.is_empty(), "{source}\n{result:#?}");
    }
}

#[test]
fn script_aliases_feed_later_item_type_literals() {
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
        41,
        "aliases:\n    building stone¦s = stone\non load:\n    send building stones\n",
    );
    let event = selected_at(&result, 1);
    let StructureBody::Trigger(body) = &event.body else {
        panic!("on load must retain its trigger: {event:#?}");
    };
    let Some(SectionBodyNode::Effect(effect)) = body.first() else {
        panic!("on load must contain the send Effect: {body:#?}");
    };

    assert!(
        effect.selected.is_some(),
        "the script alias must resolve as an ItemType literal: {effect:#?}"
    );
    assert!(result.failures.is_empty(), "{result:#?}");
}

#[test]
fn duplicate_command_labels_are_rejected_within_one_parse() {
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
        42,
        "command /same:\n    trigger:\n        dummy effect registered through wrapper\ncommand /same:\n    trigger:\n        dummy effect registered through wrapper\n",
    );

    selected_at(&result, 0);
    let StructureDocumentNode::Structure(second) = &result.document.roots[1] else {
        panic!("the duplicate must reach the Structure pipeline");
    };
    assert!(second.selected.is_none(), "{second:#?}");
    let failure = second
        .unknown
        .as_ref()
        .and_then(|unknown| unknown.failure.as_ref())
        .expect("the duplicate rejection must remain diagnosable");
    assert!(failure.root_cause().failure.reasons.iter().any(|reason| {
        matches!(reason, PatternFailureReason::HookRejected { reason }
            if reason.contains("command with the name /same is already defined"))
    }));
}

#[test]
fn example_structure_parses_its_body_as_a_trigger() {
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
        7,
        "using examples\nexample:\n    dummy effect registered through wrapper\n",
    );
    // Skript parses an Example body like a Function trigger, then discards it
    // at runtime; the parser must still retain the body for diagnostics.
    let candidate = selected_at(&result, 1);

    assert_eq!(
        candidate.element_class.as_ref().map(|value| value.as_str()),
        Some("ch.njol.skript.structures.StructExample")
    );
    assert_eq!(
        candidate
            .metadata
            .get("nlaocs.core-library/semantic-mode")
            .map(String::as_str),
        Some("example-structure")
    );
    let StructureBody::Trigger(body) = &candidate.body else {
        panic!("StructExample must parse its body as a trigger: {candidate:#?}");
    };
    let Some(SectionBodyNode::Effect(effect)) = body.first() else {
        panic!("StructExample body must contain one Effect node: {body:#?}");
    };
    assert!(
        effect.selected.is_some(),
        "the example body Effect must remain parseable: {effect:#?}"
    );
    assert!(result.document.diagnostics.is_empty());
}

#[test]
fn example_structure_requires_the_registered_examples_experiment() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            ..HostConfig::default()
        },
    )
    .unwrap();

    let disabled = parse(
        &mut host,
        8,
        "example:\n    dummy effect registered through wrapper\n",
    );
    let StructureDocumentNode::Structure(matches) = &disabled.document.roots[0] else {
        panic!("example must reach the Structure pipeline");
    };
    assert!(
        matches.selected.is_none(),
        "{matches:#?}\ncalls: {:#?}\neffects: {:#?}\nfailures: {:#?}",
        disabled.calls,
        disabled.effects,
        disabled.failures
    );

    let unknown = parse(
        &mut host,
        9,
        "using definitely-not-an-experiment\nexample:\n    dummy effect registered through wrapper\n",
    );
    let StructureDocumentNode::Structure(matches) = &unknown.document.roots[1] else {
        panic!("example must reach the Structure pipeline");
    };
    assert!(
        matches.selected.is_none(),
        "{matches:#?}\ncalls: {:#?}\neffects: {:#?}\nfailures: {:#?}",
        unknown.calls,
        unknown.effects,
        unknown.failures
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

#[test]
fn struct_event_propagates_reference_event_classes_to_trigger_effects() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            ..HostConfig::default()
        },
    )
    .unwrap();

    let join = parse(&mut host, 5, "on join:\n    send restricted event value\n");
    let join_effect = body_effect(&join);
    assert!(
        join_effect.selected.is_some(),
        "PlayerJoinEvent context must allow the restricted Expression: {join_effect:#?}"
    );
    assert!(join_effect.unknown.is_none());

    let quit = parse(&mut host, 6, "on quit:\n    send restricted event value\n");
    let quit_effect = body_effect(&quit);
    assert!(
        quit_effect.selected.is_none(),
        "PlayerQuitEvent context must reject the restricted Expression: {quit_effect:#?}"
    );
    let reason = effect_event_restriction_reason(quit_effect).unwrap_or_else(|| {
        panic!("quit failure must retain the nested EventRestricted reason: {quit_effect:#?}")
    });
    assert_eq!(reason.0, ["org.bukkit.event.player.PlayerJoinEvent"]);
    assert_eq!(reason.1, ["org.bukkit.event.player.PlayerQuitEvent"]);
}

#[test]
fn structure_enter_rejection_retries_a_later_header_candidate() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(structure_enter_rejection_fallback_catalog()),
            runtime_profile: RuntimeProfile {
                // StructVariables was introduced in the modern Structure API
                // in 2.7.  The fallback registration represents another addon
                // claiming the same header on an older server.
                skript_version: Some("2.6.4".to_owned()),
                ..RuntimeProfile::default()
            },
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let result = parse(&mut host, 50, "variables:\n");
    let candidate = selected(&result);

    assert_eq!(
        candidate.element_class.as_ref().map(ClassName::as_str),
        Some("test.GenericVariablesStructure")
    );
    assert!(!matches!(
        &result.document.roots[0],
        StructureDocumentNode::Structure(matches) if matches.unknown.is_some()
    ));
}

#[test]
fn rejected_structure_body_does_not_leak_addon_state_calls_or_context() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            ..HostConfig::default()
        },
    )
    .unwrap();
    host.load_addon(EFFECT_ADDON)
        .expect("Effect addon must load");
    let (transaction, result) = parse_with_transaction(
        &mut host,
        51,
        "on load:\n    run dummy fixture effect with \"metadata\"\n",
    );
    let effect = body_effect(&result);
    assert!(effect.selected.is_none());
    assert!(effect.unknown.is_some());

    let writes = transaction.read_write_set().unwrap().writes;
    assert!(
        writes.iter().all(|write| {
            !matches!(
                write.key.as_str(),
                "category-before" | "category-after" | "not-applicable" | "replace" | "reject"
            )
        }),
        "rejected Structure body StateStore writes leaked: {writes:#?}"
    );
    assert!(
        result
            .effects
            .context_updates
            .iter()
            .all(|update| { update.key != "reject-effects-must-be-rolled-back" })
    );
    assert!(
        result.calls.iter().all(|call| {
            !matches!(
                call.subscription_id.as_str(),
                "effect.category" | "effect.not-applicable" | "effect.replace" | "effect.reject"
            )
        }),
        "rejected Structure body hook calls leaked: {:#?}",
        result.calls
    );
    transaction.cancel().unwrap();
}

#[test]
fn entry_validator_failure_is_not_reported_as_selected_structure_success() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            ..HostConfig::default()
        },
    )
    .unwrap();
    let result = parse(&mut host, 52, "dummy rich structure \"fixture\":\n");
    let StructureDocumentNode::Structure(matches) = &result.document.roots[0] else {
        panic!("EntryValidator-backed root must reach the Structure pipeline");
    };

    assert!(
        matches.selected.is_none(),
        "an invalid EntryValidator body must not be selected: {matches:#?}"
    );
    assert!(matches.unknown.is_some());
    assert!(
        result
            .document
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == StructureDiagnosticKind::MissingRequiredEntry),
        "missing required entry must remain diagnosable: {result:#?}"
    );
}

#[test]
fn rejected_function_structure_does_not_leak_a_duplicate_declaration() {
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
        53,
        "function fixture():\n    dummy effect registered through wrapper\nfunction fixture():\n    dummy effect registered through wrapper\n",
    );

    selected_at(&result, 0);
    let StructureDocumentNode::Structure(second) = &result.document.roots[1] else {
        panic!("duplicate Function must reach the Structure pipeline");
    };
    assert!(
        second.selected.is_none(),
        "duplicate Function must be rejected"
    );
    assert_eq!(
        result.functions.registrations().len(),
        1,
        "the rejected Function declaration must not leak into the registry: {result:#?}"
    );
}

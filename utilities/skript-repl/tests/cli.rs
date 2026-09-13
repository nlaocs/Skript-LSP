use serde_json::Value;
use skript_repl::{
    AnalysisReport, EXIT_NO_MATCH, EXIT_SUCCESS, OutputFormat, SkriptSession, run_with_io,
};
use std::ffi::OsString;
use std::io::Cursor;
use std::path::{Path, PathBuf};

fn modern_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn legacy_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ssg/tests/data/legacy-2.6.4-mc-1.12.2")
}

fn type_parser_216_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parser-wasm/tests/data/type-parser-versions/skript-2.16.0")
}

fn text_macro_addon() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/text-macro-addon.wasm")
}

fn arguments(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn render_json_and_human(report: AnalysisReport) -> (Value, String) {
    let json = serde_json::from_str(&report.clone().to_json().unwrap()).unwrap();
    let mut human = Vec::new();
    report.write(OutputFormat::Human, &mut human).unwrap();
    (json, String::from_utf8(human).unwrap())
}

fn document_json(report: skript_repl::DocumentAnalysisReport) -> Value {
    serde_json::from_str(&report.to_json().unwrap()).unwrap()
}

#[test]
fn parses_a_multiline_event_with_nested_sections_as_one_document_tree() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");
    let source = concat!(
        "on join:\n",
        "    send \"hello\" to console\n",
        "    loop all players:\n",
        "        send loop-player's name to console\n",
        "    send stone\n",
    );
    let report = session
        .analyze_document(source)
        .expect("the shared document parser must complete");
    assert!(report.matched(), "{}", report.clone().to_json().unwrap());
    let json = document_json(report);

    assert_eq!(json["schemaVersion"], 1);
    assert_eq!(json["input"], source);
    assert_eq!(json["status"], "matched");
    assert!(json["recoveries"].as_array().unwrap().is_empty());
    assert!(json["diagnostics"].as_array().unwrap().is_empty());

    let nodes = json["nodes"].as_array().unwrap();
    for kind in [
        "Event",
        "Effect",
        "Expression",
        "Type",
        "Section",
        "Structure",
    ] {
        assert!(
            nodes.iter().any(|node| node["kind"] == kind),
            "missing {kind} node in {nodes:?}"
        );
    }
    let root_id = json["roots"][0].as_u64().unwrap();
    let root = nodes.iter().find(|node| node["id"] == root_id).unwrap();
    assert_eq!(root["kind"], "Structure");
    assert!(
        root["identity"]["elementClass"]
            .as_str()
            .unwrap()
            .ends_with("StructEvent")
    );
    let root_children = root["children"].as_array().unwrap();
    assert!(root_children.len() >= 4);

    let section = nodes.iter().find(|node| node["kind"] == "Section").unwrap();
    assert!(section["children"].as_array().unwrap().iter().any(|child| {
        let id = child.as_u64().unwrap();
        nodes
            .iter()
            .any(|node| node["id"] == id && node["kind"] == "Effect")
    }));
    assert!(nodes.iter().any(|node| {
        node["kind"] == "Type"
            && node["text"] == "\"hello\""
            && node["identity"]["syntaxId"] == "string"
    }));
    assert!(
        nodes
            .iter()
            .any(|node| { node["semantics"]["defaultExpression"].as_object().is_some() })
    );
    assert!(nodes.iter().any(|node| {
        node["contextOrigin"] == "preserved"
            && node["span"]["origins"]
                .as_array()
                .is_some_and(|origins| origins.iter().any(|origin| origin["kind"] == "exact"))
    }));
    assert!(nodes.iter().any(|node| {
        node["semantics"]["possibleReturnTypesState"] == "complete"
            && node["semantics"]["multiplicity"] == "single"
    }));
}

#[test]
fn document_report_preserves_text_macro_source_provenance_and_state_accesses() {
    let addon = text_macro_addon();
    assert!(
        addon.is_file(),
        "build test components before running this test"
    );
    let mut session = SkriptSession::load_with_addons(type_parser_216_fixture(), [&addon])
        .expect("fixture and Text macro addon must load");
    let report = session
        .analyze_document("alpha")
        .expect("macro-expanded document must produce a partial report");
    let mut human = Vec::new();
    report.write(OutputFormat::Human, &mut human).unwrap();
    let json = document_json(report);

    assert_eq!(json["input"], "alpha");
    assert_eq!(json["virtualSource"], "二段目");
    assert_eq!(
        json["macroPipeline"]["text"]["decision"]["kind"],
        "continueProcessing"
    );
    let calls = json["macroPipeline"]["text"]["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|call| call["accepted"] == true));
    assert_eq!(calls[0]["stateAccesses"]["writes"][0]["key"], "text.first");
    assert_eq!(calls[1]["stateAccesses"]["writes"][0]["key"], "text.second");
    assert_eq!(
        json["macroPipeline"]["expansions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let diagnostic = json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == "fixture.generated-source")
        .expect("macro diagnostic must be retained");
    assert_eq!(
        diagnostic["span"]["virtualRange"],
        serde_json::json!({"start": 0, "end": 9})
    );
    assert_eq!(
        diagnostic["span"]["origins"][0]["originalRange"],
        serde_json::json!({"start": 0, "end": 5})
    );
    let human = String::from_utf8(human).unwrap();
    assert!(human.contains("expandedSource:"));
    assert!(human.contains("alpha"));
    assert!(human.contains("diagnostic over a prior macro expansion"));
}

#[test]
fn nested_unclaimed_lines_remain_document_recoveries() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");
    let report = session
        .analyze_document("on join:\n \tdefinitely not syntax\n    send \"after\" to console\n")
        .expect("an unclaimed nested line must remain recoverable");
    let json = document_json(report);

    assert_eq!(json["status"], "incomplete");
    assert!(
        json["recoveries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|recovery| {
                recovery["kind"] == "RawNode" && recovery["source"] == "definitely not syntax"
            })
    );
}

#[test]
fn human_document_report_explains_section_semantic_rejection() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");
    let report = session
        .analyze_document("on load:\n    catch runtime errors:\n        send 1 to console\n")
        .expect("a rejected Section must remain recoverable");
    let mut output = Vec::new();
    report.write(OutputFormat::Human, &mut output).unwrap();
    let output = String::from_utf8(output).unwrap();

    assert!(
        output.contains("Section candidate is incomplete"),
        "{output}"
    );
    assert!(
        output.contains("the `catch runtime errors` experiment is not enabled"),
        "{output}"
    );
    assert!(
        output.contains("Section pattern: catch [run[ ]time] error[s]"),
        "{output}"
    );
    assert!(
        !output.contains("Section was not claimed by any registered syntax"),
        "{output}"
    );
}

#[test]
fn document_report_keeps_default_expression_rejection_inside_an_event() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");
    let report = session
        .analyze_document("on weather change:\n    send stone\n")
        .expect("default rejection must leave a partial document");
    let json = document_json(report);

    assert_eq!(json["status"], "incomplete");
    assert!(
        json["recoveries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|recovery| {
                recovery["failure"]["reasons"]
                    .as_array()
                    .is_some_and(|reasons| {
                        reasons.iter().any(|reason| {
                            reason["kind"] == "defaultExpression" && reason["state"] == "rejected"
                        })
                    })
            })
    );
}

#[test]
fn multiline_document_keeps_partial_tree_and_source_spans_for_invalid_children() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");
    let source = "on join:\n    teleport a to location(b, 2, 3)\n    send \"after\" to console\n";
    let report = session
        .analyze_document(source)
        .expect("recoverable syntax failures must still produce a report");
    assert!(!report.matched());
    let json = document_json(report);

    assert_eq!(json["input"], source);
    assert_eq!(json["status"], "incomplete");
    assert!(
        json["roots"]
            .as_array()
            .is_some_and(|roots| !roots.is_empty())
    );
    assert!(
        json["nodes"].as_array().unwrap().iter().any(|node| {
            node["kind"] == "Effect" && node["text"] == "send \"after\" to console"
        })
    );
    assert!(
        json["recoveries"]
            .as_array()
            .is_some_and(|recoveries| !recoveries.is_empty())
            || json["diagnostics"]
                .as_array()
                .is_some_and(|diagnostics| !diagnostics.is_empty())
    );
    for diagnostic in json["diagnostics"].as_array().unwrap() {
        assert!(
            diagnostic["span"]["virtualRange"]["end"].as_u64().unwrap()
                <= u64::try_from(source.len()).unwrap()
        );
    }
}

#[test]
fn multiline_documents_are_self_contained_and_preserve_manual_repl_context() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");
    session
        .select_event_header("on join")
        .expect("manual Event context must parse");

    let document = session
        .analyze_document("on weather change:\n    send player's name to console\n")
        .expect("the self-contained document must produce a partial result");
    assert!(
        !document.matched(),
        "a weather Event must not inherit the manually selected join Event"
    );
    assert_eq!(
        session.event_context().map(|event| event.input.as_str()),
        Some("on join")
    );

    let one_line = session
        .analyze_effect("send player's name to console")
        .expect("the preserved manual Event context must remain usable");
    assert!(one_line.matched());
}

#[test]
fn stream_repl_submits_multiline_source_then_accepts_another_effect() {
    let snapshot = type_parser_216_fixture();
    let input = Cursor::new(
        b"on join:\n    send \"inside\" to console\n\nsend \"after\" to console\n:quit\n".to_vec(),
    );
    let mut output = Vec::new();
    let mut error = Vec::new();
    let code = run_with_io(
        arguments(&["--snapshot", snapshot.to_str().unwrap(), "--repl"]),
        PathBuf::from("unused"),
        input,
        &mut output,
        &mut error,
    );

    assert_eq!(code, EXIT_SUCCESS);
    assert!(error.is_empty());
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Skript REPL"));
    assert!(output.contains("skript> "));
    assert!(output.contains("......> "));
    assert!(output.contains("  1 | on join:"));
    assert!(output.contains("  2 |     send \"inside\" to console"));
    assert!(output.contains("document: matched"));
    assert!(output.contains("StructEvent"));
    assert!(output.contains("send \"after\" to console"));
}

#[test]
fn stream_repl_cancel_discards_a_draft_and_eof_submits_the_next_one() {
    let snapshot = type_parser_216_fixture();
    let input = Cursor::new(
        b"on join:\n    send \"discarded\" to console\n:cancel\non join:\n    send \"submitted\" to console\n"
            .to_vec(),
    );
    let mut output = Vec::new();
    let mut error = Vec::new();
    let code = run_with_io(
        arguments(&["--snapshot", snapshot.to_str().unwrap(), "--repl"]),
        PathBuf::from("unused"),
        input,
        &mut output,
        &mut error,
    );

    assert_eq!(code, EXIT_SUCCESS);
    assert!(error.is_empty());
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("multiline input discarded"));
    assert!(!output.contains("send \"discarded\" to console"));
    assert!(output.contains("send \"submitted\" to console"));
    assert_eq!(output.matches("document: matched").count(), 1);
}

#[test]
fn default_expression_without_event_reports_the_omitted_audience() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");
    let report = session.analyze_effect("send stone").unwrap();
    assert!(!report.matched());
    let (json, human) = render_json_and_human(report);

    assert_eq!(json["schemaVersion"], 7);
    assert_eq!(json["input"], "send stone");
    assert!(json["context"]["event"].is_null());
    assert_eq!(json["result"]["status"], "incomplete");
    assert_eq!(
        json["result"]["effect"]["syntax"]["elementClass"],
        "org.skriptlang.skript.bukkit.text.elements.effects.EffMessage"
    );
    assert_eq!(
        json["result"]["effect"]["pattern"],
        "(message|send [message[s]]) %objects% [to %audiences%]"
    );
    let failure = &json["result"]["failure"];
    assert_eq!(failure["span"], serde_json::json!({"start": 10, "end": 10}));
    let reason = &failure["reasons"][0];
    assert_eq!(reason["kind"], "defaultExpression");
    assert_eq!(reason["captureIndex"], 1);
    assert_eq!(reason["state"], "rejected");
    assert_eq!(reason["expected"], serde_json::json!(["audience[]"]));
    assert_eq!(
        reason["reason"],
        "omitted audience requires an Event providing org.bukkit.command.CommandSender; no Event context is active"
    );
    assert!(human.contains("Effect candidate is incomplete"));
    assert!(human.contains("EffMessage"));
    assert!(human.contains("[to %audiences%]"));
    let human_words = human.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(human_words.contains("omitted capture #1 (audience[]) default rejected"));
    assert!(human_words.contains(reason["reason"].as_str().unwrap()));
    assert!(
        human
            .lines()
            .any(|line| line.trim_end().ends_with("send stone"))
    );
    assert!(!human.contains("send stone to"));
}

#[test]
fn default_expression_on_join_is_an_implicit_child_with_an_anchor() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");
    session.select_event_header("on join").unwrap();
    let report = session.analyze_effect("send stone").unwrap();
    assert!(report.matched(), "{}", report.clone().to_json().unwrap());
    let (json, human) = render_json_and_human(report);

    assert_eq!(json["input"], "send stone");
    assert_eq!(json["result"]["status"], "matched");
    let captures = json["result"]["effect"]["elements"].as_array().unwrap();
    let message = captures
        .iter()
        .find(|value| value["captureIndex"] == 0)
        .unwrap();
    assert_eq!(message["state"], "explicit");
    assert_eq!(message["source"], "stone");
    let recipient = captures
        .iter()
        .find(|value| value["captureIndex"] == 1)
        .unwrap();
    assert_eq!(recipient["state"], "default");
    assert_eq!(recipient["source"], "");
    assert_eq!(
        recipient["span"],
        serde_json::json!({"start": 10, "end": 10})
    );
    assert_eq!(
        recipient["expected"]["alternatives"][0]["codeName"],
        "audience"
    );
    let resolved = &recipient["resolved"];
    assert_eq!(resolved["source"], "");
    assert_eq!(resolved["span"], recipient["span"]);
    assert_eq!(resolved["expression"]["kind"], "default");
    assert_eq!(resolved["returnType"], "org.bukkit.command.CommandSender");
    assert_eq!(resolved["multiplicity"], "single");
    assert_eq!(
        resolved["metadata"]["nlaocs.core-library/default-expression-class"],
        "ch.njol.skript.expressions.base.EventValueExpression"
    );
    let default = &resolved["defaultExpression"];
    assert_eq!(default["implicit"], true);
    assert_eq!(default["captureIndex"], 1);
    assert_eq!(default["expression"], "%audiences%");
    assert_eq!(default["patternSpan"], recipient["patternSpan"]);
    assert_eq!(default["providerId"], "core.default-expression.skript");
    assert_eq!(default["componentId"], "nlaocs.core-library");
    assert_eq!(
        default["requestedType"],
        serde_json::json!({"className": "net.kyori.adventure.audience.Audience", "plural": true})
    );
    assert_eq!(default["isLiteral"], false);
    assert_eq!(default["time"], 0);
    assert_eq!(
        default["context"]["eventClasses"],
        serde_json::json!(["org.bukkit.event.player.PlayerJoinEvent"])
    );
    assert_eq!(default["context"]["sectionScopeIds"], serde_json::json!([]));
    assert_eq!(default["anchor"]["start"], 10);
    assert_eq!(default["anchor"]["end"], 10);
    let origins = default["anchor"]["origins"].as_array().unwrap();
    assert!(!origins.is_empty());
    for origin in origins {
        assert_eq!(origin["kind"], "exact");
        assert_eq!(origin["start"], 10);
        assert_eq!(origin["end"], 10);
    }
    let references = default["catalogReferences"].as_array().unwrap();
    let type_reference = references
        .iter()
        .find(|value| value["role"] == "type")
        .unwrap();
    assert!(
        default["typeDefinitionId"]
            .as_str()
            .unwrap()
            .starts_with("type:skript:")
    );
    assert_eq!(type_reference["definitionId"], default["typeDefinitionId"]);
    assert_eq!(
        type_reference["registrationId"],
        default["typeRegistrationId"]
    );
    assert_eq!(type_reference["snapshotId"], json["snapshot"]["id"]);
    assert_eq!(type_reference["document"], "Types.json");
    let event_value = references
        .iter()
        .find(|value| value["role"] == "event-value")
        .unwrap();
    assert_eq!(event_value["snapshotId"], json["snapshot"]["id"]);
    assert_eq!(event_value["document"], "EventValues.json");
    assert!(event_value["index"].is_u64());
    assert!(
        json["context"]["event"]["eventValues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| {
                value["registrationId"] == event_value["registrationId"]
                    && value["valueClass"] == "org.bukkit.entity.Player"
            })
    );
    assert!(human.contains("expression \"\" at 10..10 (capture #1, default)"));
    assert!(human.contains(
        "resolved: implicit / default (nlaocs.core-library/core.default-expression.skript)"
    ));
    assert!(human.contains(default["reason"].as_str().unwrap()));
    assert!(
        human
            .lines()
            .any(|line| line.trim() == "source: send stone")
    );
    assert!(!human.contains("to player"));
}

#[test]
fn default_expression_preserves_an_explicit_console_without_event_context() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");
    let report = session.analyze_effect("send stone to console").unwrap();
    assert!(report.matched());
    let (json, human) = render_json_and_human(report);

    assert!(json["context"]["event"].is_null());
    assert_eq!(json["input"], "send stone to console");
    let recipient = json["result"]["effect"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["captureIndex"] == 1)
        .unwrap();
    assert_eq!(recipient["state"], "explicit");
    assert_eq!(recipient["source"], "console");
    assert_eq!(
        recipient["span"],
        serde_json::json!({"start": 14, "end": 21})
    );
    assert_eq!(recipient["resolved"]["expression"]["kind"], "registered");
    assert_eq!(
        recipient["resolved"]["expression"]["syntax"]["elementClass"],
        "ch.njol.skript.literals.LitConsole"
    );
    assert!(recipient["resolved"].get("defaultExpression").is_none());
    assert!(!human.contains("implicit / default"));
    assert!(
        human
            .lines()
            .any(|line| line.trim() == "source: send stone to console")
    );
}

#[test]
fn default_expression_rejects_weather_events_that_only_provide_a_world() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");
    session.select_event_header("on weather change").unwrap();
    let report = session.analyze_effect("send stone").unwrap();
    assert!(!report.matched());
    let (json, human) = render_json_and_human(report);

    assert_eq!(json["result"]["status"], "incomplete");
    assert_eq!(
        json["result"]["effect"]["syntax"]["elementClass"],
        "org.skriptlang.skript.bukkit.text.elements.effects.EffMessage"
    );
    let values = json["context"]["event"]["eventValues"].as_array().unwrap();
    assert!(!values.is_empty());
    assert!(
        values
            .iter()
            .all(|value| value["valueClass"] == "org.bukkit.World")
    );
    let reason = &json["result"]["failure"]["reasons"][0];
    assert_eq!(reason["kind"], "defaultExpression");
    assert_eq!(reason["captureIndex"], 1);
    assert_eq!(reason["state"], "rejected");
    assert_eq!(reason["expected"], serde_json::json!(["audience[]"]));
    assert!(reason["reason"].as_str().unwrap().starts_with(
        "omitted audience requires org.bukkit.command.CommandSender; the current Event provides none"
    ));
    let human_words = human.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(human_words.contains("omitted capture #1 (audience[]) default rejected"));
    assert!(human_words.contains(reason["reason"].as_str().unwrap()));
}

#[test]
fn legacy_default_expression_without_static_shape_is_unresolved() {
    let mut session = SkriptSession::load(modern_fixture()).expect("fixture must load");
    session.select_event_header("on join").unwrap();
    let report = session.analyze_effect("send 1").unwrap();
    assert!(!report.matched());
    let (json, human) = render_json_and_human(report);

    assert_eq!(json["result"]["status"], "incomplete");
    assert_eq!(
        json["result"]["effect"]["syntax"]["elementClass"],
        "org.skriptlang.skript.bukkit.text.elements.effects.EffMessage"
    );
    let reason = &json["result"]["failure"]["reasons"][0];
    assert_eq!(reason["kind"], "defaultExpression");
    assert_eq!(reason["captureIndex"], 1);
    assert_eq!(reason["state"], "unresolved");
    assert_eq!(reason["expected"], serde_json::json!(["audience[]"]));
    assert_eq!(
        reason["reason"],
        "audience DefaultExpression has no statically verified return type"
    );
    let human_words = human.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(human_words.contains("omitted capture #1 (audience[]) default unresolved"));
    assert!(human_words.contains(reason["reason"].as_str().unwrap()));
}

#[test]
fn parses_effect_and_reports_literal_and_type_information() {
    let mut session = SkriptSession::load(modern_fixture()).expect("fixture must load");
    let report = session
        .analyze_effect("send 1 to console")
        .expect("Effect must parse");
    assert!(report.matched());

    let json_text = report.to_json().unwrap();
    assert!(!json_text.contains('\x1b'));
    let json: Value = serde_json::from_str(&json_text).unwrap();
    assert_eq!(json["schemaVersion"], 7);
    assert!(json["context"]["event"].is_null());
    assert!(json["parseDurationNs"].is_u64());
    assert_eq!(json["result"]["status"], "matched");
    assert_eq!(
        json["result"]["effect"]["syntax"]["elementClass"],
        "org.skriptlang.skript.bukkit.text.elements.effects.EffMessage"
    );
    let elements = json["result"]["effect"]["elements"]
        .as_array()
        .expect("Effect captures are an array");
    let expression = elements
        .iter()
        .find(|element| element["kind"] == "expression")
        .expect("send captures the message Expression");
    assert_eq!(expression["source"], "1");
    assert_eq!(expression["selectedAlternative"], 0);
    assert!(expression.get("selected_alternative").is_none());
    assert!(expression["patternSpan"].is_object());
    assert_eq!(
        expression["expected"]["alternatives"][0]["codeName"],
        "object"
    );
    assert_eq!(expression["resolved"]["expression"]["kind"], "literal");
    assert_eq!(
        expression["resolved"]["expression"]["parserId"],
        "core.literal.number"
    );
    assert_eq!(expression["resolved"]["returnType"], "java.lang.Long");

    let addon_report = session
        .analyze_effect("dummy effect registered through wrapper")
        .expect("DummyAddon Effect must parse");
    let addon_json: Value = serde_json::from_str(&addon_report.to_json().unwrap()).unwrap();
    assert_eq!(
        addon_json["result"]["effect"]["syntax"]["addon"]["name"],
        "SkriptDummyAddon"
    );
    assert_eq!(
        addon_json["result"]["effect"]["syntax"]["elementClass"],
        "jp.nlaocs.skriptDummyAddon.fixture.LegacySyntaxes$WrappedEffect"
    );
}

#[test]
fn reports_parenthesized_expression_and_its_inner_span() {
    let snapshot = modern_fixture();
    let mut session = SkriptSession::load(&snapshot).expect("fixture must load");
    let report = session
        .analyze_effect("send (1) to console")
        .expect("parenthesized Expression must parse");
    assert!(report.matched());

    let json: Value = serde_json::from_str(&report.to_json().unwrap()).unwrap();
    let grouped = &json["result"]["effect"]["elements"][0]["resolved"];
    assert_eq!(grouped["expression"]["kind"], "grouped");
    assert_eq!(grouped["source"], "(1)");
    assert_eq!(grouped["span"]["start"], 5);
    assert_eq!(grouped["span"]["end"], 8);
    assert_eq!(grouped["inner"]["expression"]["kind"], "literal");
    assert_eq!(grouped["inner"]["source"], "1");
    assert_eq!(grouped["inner"]["span"]["start"], 6);
    assert_eq!(grouped["inner"]["span"]["end"], 7);

    let mut output = Vec::new();
    let mut error = Vec::new();
    let code = run_with_io(
        arguments(&[
            "--snapshot",
            snapshot.to_str().unwrap(),
            "send (1) to console",
        ]),
        PathBuf::from("unused"),
        Cursor::new(Vec::<u8>::new()),
        &mut output,
        &mut error,
    );
    assert_eq!(code, EXIT_SUCCESS);
    assert!(error.is_empty());
    let human = String::from_utf8(output).unwrap();
    assert!(human.contains("resolved: groupedExpression"));
    assert!(human.contains("inner:"));
}

#[test]
fn parses_enchanted_item_type_before_eff_change_delimiter() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");
    session
        .select_event_header("on join")
        .expect("player target needs an Event context");

    let report = session
        .analyze_effect("give a diamond sword of sharpness to player")
        .expect("Effect analysis must complete");
    assert!(report.matched());

    let json: Value = serde_json::from_str(&report.to_json().unwrap()).unwrap();
    let item = &json["result"]["effect"]["elements"][0]["resolved"];
    assert_eq!(item["expression"]["parserId"], "core.literal.item-type");
    assert_eq!(item["source"], "a diamond sword of sharpness");
    assert_eq!(
        item["metadata"]["nlaocs.core-library/literal-canonical"],
        "diamond sword"
    );
    assert_eq!(
        item["metadata"]["nlaocs.core-library/literal-enchantment.0.name"],
        "sharpness"
    );
}

#[test]
fn parses_composite_standard_type_literals_in_effects() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");
    session
        .select_event_header("on join")
        .expect("player expressions need an Event context");

    for source in [
        "give 10 xp to player",
        "draw 3 flame particle at location(1,2,3)",
        "send 12:00 to console",
        "send day to console",
        "send 1 to console if {_a} is a number",
    ] {
        let report = session
            .analyze_effect(source)
            .unwrap_or_else(|error| panic!("Effect analysis failed for {source:?}: {error}"));
        assert!(
            report.matched(),
            "{source:?} must match:\n{}",
            report.to_json().unwrap()
        );
    }
}

#[test]
fn reports_node_local_public_data_as_structured_json() {
    let snapshot = modern_fixture();
    let mut session = SkriptSession::load(&snapshot).expect("fixture must load");
    let report = session
        .analyze_effect("send ({_money}) to console")
        .expect("grouped variable Expression must parse");
    assert!(report.matched());

    let json: Value = serde_json::from_str(&report.to_json().unwrap()).unwrap();
    let grouped = &json["result"]["effect"]["elements"][0]["resolved"];
    assert_eq!(grouped["source"], "({_money})");
    assert_eq!(grouped["publicData"], serde_json::json!([]));

    let variable = &grouped["inner"];
    assert_eq!(variable["source"], "{_money}");
    let public_data = &variable["publicData"][0];
    assert_eq!(public_data["schemaId"], "nlaocs.skript.variable");
    assert_eq!(public_data["schemaVersion"], 1);
    assert_eq!(
        public_data["json"],
        serde_json::json!({
            "scope": "local",
            "name": [{"kind": "text", "text": "money"}],
        })
    );

    let escaped = session
        .analyze_effect("send {_literal%%percent} to console")
        .expect("escaped percent variable Expression must parse");
    let escaped_json: Value = serde_json::from_str(&escaped.to_json().unwrap()).unwrap();
    let escaped_variable = &escaped_json["result"]["effect"]["elements"][0]["resolved"];
    assert_eq!(escaped_variable["source"], "{_literal%%percent}");
    assert_eq!(
        escaped_variable["publicData"][0]["json"]["name"][0]["text"],
        "literal%%percent"
    );

    let mut output = Vec::new();
    let mut error = Vec::new();
    let code = run_with_io(
        arguments(&[
            "--snapshot",
            snapshot.to_str().unwrap(),
            "--json",
            "send {_money} to console",
        ]),
        PathBuf::from("unused"),
        Cursor::new(Vec::<u8>::new()),
        &mut output,
        &mut error,
    );
    assert_eq!(code, EXIT_SUCCESS);
    assert!(error.is_empty());
    let cli_json: Value = serde_json::from_slice(&output).unwrap();
    let cli_public_data = &cli_json["result"]["effect"]["elements"][0]["resolved"]["publicData"][0];
    assert!(cli_public_data["json"].is_object());
    assert_eq!(cli_public_data["schemaId"], "nlaocs.skript.variable");

    output.clear();
    error.clear();
    let code = run_with_io(
        arguments(&[
            "--snapshot",
            snapshot.to_str().unwrap(),
            "send {_money} to console",
        ]),
        PathBuf::from("unused"),
        Cursor::new(Vec::<u8>::new()),
        &mut output,
        &mut error,
    );
    assert_eq!(code, EXIT_SUCCESS);
    assert!(error.is_empty());
    let human = String::from_utf8(output).unwrap();
    assert!(human.contains("source: send {_money} to console"));
    assert!(human.contains("publicData:"));
    assert!(human.contains("schemaId: nlaocs.skript.variable"));
    assert!(human.contains("schemaVersion: 1"));
    assert!(human.contains("json: {"));
    assert!(human.contains("\"money\""));
}

#[test]
fn reports_interpolated_variable_public_data_and_embedded_children() {
    let mut session = SkriptSession::load(modern_fixture()).expect("fixture must load");
    let report = session
        .analyze_effect("send {_price::%{_key}%} to console")
        .expect("interpolated variable Expression must parse");
    assert!(report.matched());

    let json: Value = serde_json::from_str(&report.to_json().unwrap()).unwrap();
    let variable = &json["result"]["effect"]["elements"][0]["resolved"];
    assert_eq!(variable["source"], "{_price::%{_key}%}");
    assert_eq!(
        variable["publicData"][0]["json"]["name"],
        serde_json::json!([
            {"kind": "text", "text": "price::"},
            {"kind": "expression", "childIndex": 0},
        ])
    );
    assert_eq!(variable["embeddedExpressions"].as_array().unwrap().len(), 1);
    let embedded = &variable["embeddedExpressions"][0];
    assert_eq!(embedded["source"], "{_key}");
    assert_eq!(
        embedded["publicData"][0]["schemaId"],
        "nlaocs.skript.variable"
    );
}

#[test]
fn reports_registered_function_identity_and_arguments() {
    let snapshot = modern_fixture();
    let mut session = SkriptSession::load(&snapshot).expect("fixture must load");
    let report = session
        .analyze_effect("send sin(abs(-1)) to console")
        .expect("nested Function Effect must parse");
    assert!(report.matched());

    let json: Value = serde_json::from_str(&report.to_json().unwrap()).unwrap();
    let function = &json["result"]["effect"]["elements"][0]["resolved"];
    assert_eq!(function["expression"]["kind"], "function");
    assert_eq!(function["expression"]["parserId"], "core.function");
    assert_eq!(function["expression"]["structured"], true);
    assert_eq!(function["expression"]["name"], "sin");
    assert_eq!(function["expression"]["syntax"]["addon"]["name"], "Skript");
    assert_eq!(function["arguments"][0]["parameterName"], "n");
    assert_eq!(
        function["arguments"][0]["values"][0]["expression"]["name"],
        "abs"
    );
    assert_eq!(
        function["arguments"][0]["values"][0]["arguments"][0]["values"][0]["returnType"],
        "java.lang.Long"
    );

    let mut output = Vec::new();
    let mut error = Vec::new();
    let code = run_with_io(
        arguments(&[
            "--snapshot",
            snapshot.to_str().unwrap(),
            "send log(8) to console",
        ]),
        PathBuf::from("unused"),
        Cursor::new(Vec::<u8>::new()),
        &mut output,
        &mut error,
    );
    assert_eq!(code, EXIT_SUCCESS);
    assert!(error.is_empty());
    let human = String::from_utf8(output).unwrap();
    assert!(human.contains("resolved: function (core.function, structured=true)"));
    assert!(human.contains("name: log"));
    assert!(human.contains("base:"));
    assert!(human.contains("omitted: true"));

    let mut legacy = SkriptSession::load(legacy_fixture()).expect("legacy fixture must load");
    let legacy: Value = serde_json::from_str(
        &legacy
            .analyze_effect("send sin(1) to console")
            .expect("2.6.4 Function Effect must parse")
            .to_json()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(legacy["result"]["status"], "matched");
    assert_eq!(
        legacy["result"]["effect"]["elements"][0]["resolved"]["expression"]["syntax"]["addon"]["version"],
        "2.6.4"
    );
}

#[test]
fn reports_embedded_registered_expression_inside_variable_string() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");
    let report = session
        .analyze_effect(r#"send "players: %size of all players%" to console"#)
        .expect("variable-string Expression must parse");
    assert!(report.matched());

    let json: Value = serde_json::from_str(&report.to_json().unwrap()).unwrap();
    let outer = &json["result"]["effect"]["elements"][0]["resolved"];
    let embedded = &outer["embeddedExpressions"][0];
    assert_eq!(embedded["expression"]["kind"], "registered");
    assert_eq!(
        embedded["expression"]["syntax"]["elementClass"],
        "org.skriptlang.skript.common.properties.elements.expressions.PropExprSize"
    );
    assert_eq!(embedded["source"], "size of all players");

    let elements = embedded["elements"]
        .as_array()
        .expect("PropExprSize captures must be an array");
    assert!(
        elements.iter().any(|element| {
            element["kind"] == "expression" && element["source"] == "all players"
        })
    );
}

#[test]
fn reports_arithmetic_operations_and_operands() {
    let snapshot = modern_fixture();
    let mut session = SkriptSession::load(&snapshot).expect("fixture must load");
    let report = session
        .analyze_effect("return 1 + 2 * 3")
        .expect("arithmetic Effect must parse");
    assert!(report.matched());

    let json: Value = serde_json::from_str(&report.to_json().unwrap()).unwrap();
    let arithmetic = &json["result"]["effect"]["elements"][0]["resolved"];
    assert_eq!(arithmetic["expression"]["kind"], "arithmetic");
    assert_eq!(arithmetic["expression"]["operator"], "+");
    assert_eq!(arithmetic["expression"]["addon"]["name"], "Skript");
    assert_eq!(arithmetic["operands"][0]["source"], "1");
    assert_eq!(arithmetic["operands"][1]["expression"]["operator"], "*");

    let mut output = Vec::new();
    let mut error = Vec::new();
    let code = run_with_io(
        arguments(&["--snapshot", snapshot.to_str().unwrap(), "return 1 + 2 * 3"]),
        PathBuf::from("unused"),
        Cursor::new(Vec::<u8>::new()),
        &mut output,
        &mut error,
    );
    assert_eq!(code, EXIT_SUCCESS);
    assert!(error.is_empty());
    let human = String::from_utf8(output).unwrap();
    assert!(human.contains("parseTime:"));
    assert!(human.contains("source: return 1 + 2 * 3"));
    assert!(!human.contains('\x1b'));
    assert!(human.contains("resolved: arithmetic (+)"));
    assert!(human.contains("operands:"));
}

#[test]
fn parses_boolean_conditions_and_item_alias_literals() {
    let mut session = SkriptSession::load(modern_fixture()).expect("fixture must load");
    for source in ["send 2 to console if true is true", "send stone to console"] {
        let report = session
            .analyze_effect(source)
            .expect("Effect analysis must complete");
        assert!(report.matched(), "{source:?} must parse");
    }

    let invalid_comparison = session
        .analyze_effect("send 1 to console if 1 is true")
        .expect("invalid comparison analysis must complete");
    assert!(!invalid_comparison.matched());
    let json = invalid_comparison.to_json().unwrap();
    assert!(
        json.contains("cannot compare java.lang.Long with java.lang.Boolean"),
        "native Skript rejects the same incompatible comparison: {json}"
    );
}

#[test]
fn reports_nested_condition_failure_as_incomplete_effect_candidate() {
    let snapshot = modern_fixture();
    let mut output = Vec::new();
    let mut error = Vec::new();
    let code = run_with_io(
        arguments(&[
            "--snapshot",
            snapshot.to_str().unwrap(),
            "--json",
            "send 2 to console if true",
        ]),
        PathBuf::from("unused"),
        Cursor::new(Vec::<u8>::new()),
        &mut output,
        &mut error,
    );
    assert_eq!(code, EXIT_NO_MATCH);
    assert!(error.is_empty());

    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["result"]["status"], "incomplete");
    assert_eq!(
        json["result"]["effect"]["syntax"]["elementClass"],
        "ch.njol.skript.effects.EffDoIf"
    );
    assert_eq!(json["result"]["failure"]["span"]["start"], 21);
    assert_eq!(json["result"]["failure"]["span"]["end"], 25);
    assert_eq!(
        json["result"]["failure"]["reasons"][0]["kind"],
        "trailingInput"
    );
}

#[test]
fn renders_human_failures_with_a_source_label() {
    let mut output = Vec::new();
    let mut error = Vec::new();
    let code = run_with_io(
        arguments(&[
            "--snapshot",
            modern_fixture().to_str().unwrap(),
            "send 2 to console if true",
        ]),
        PathBuf::from("unused"),
        Cursor::new(Vec::<u8>::new()),
        &mut output,
        &mut error,
    );

    assert_eq!(code, EXIT_NO_MATCH);
    assert!(error.is_empty());
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Effect candidate is incomplete"));
    assert!(output.contains("send 2 to console if true"));
    assert!(output.contains("unexpected trailing input"));
    assert!(!output.contains('\x1b'));
}

#[test]
fn reports_nested_root_cause_patterns_and_competing_effect_interpretations() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");
    session
        .select_event_header("on join")
        .expect("the competing EffDoIf interpretation has an omitted audience");
    let report = session
        .analyze_effect("send 1 if a < 5 else 2")
        .expect("invalid nested condition is a recoverable no-match");
    let json: Value = serde_json::from_str(&report.to_json().unwrap()).unwrap();

    assert_eq!(
        json["result"]["effect"]["syntax"]["elementClass"],
        "org.skriptlang.skript.bukkit.text.elements.effects.EffMessage"
    );
    assert_eq!(json["result"]["failure"]["span"]["start"], 10);
    assert_eq!(json["result"]["failure"]["span"]["end"], 11);
    let contexts = json["result"]["failure"]["contexts"].as_array().unwrap();
    assert!(contexts.iter().any(|context| {
        context["pattern"] == "%objects% if <.+>[,] (otherwise|else) %objects%"
    }));
    assert!(
        json["result"]["failure"]["interpretations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|interpretation| interpretation["pattern"] == "<.+> if <.+>")
    );

    let report = session
        .analyze_effect("send 1 if a < 5 else 2")
        .expect("repeated analysis must remain deterministic");
    let mut output = Vec::new();
    report
        .write(OutputFormat::Human, &mut output)
        .expect("human report must render");
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("expected expression of type object"));
    assert!(output.contains("Expression pattern: %objects% if <.+>[,] (otherwise|else) %objects%"));
    assert!(output.contains("also considered ch.njol.skript.effects.EffDoIf pattern"));
    assert!(output.contains("if \"a\" is a variable, write {a}"));
    assert!(!output.contains("expected literal \"neither\""));
}

#[test]
fn reports_event_restrictions_and_parses_interface_expressions() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");

    let absorbed = session
        .analyze_effect("send absorbed blocks to console")
        .expect("missing Event context must be a normal no-match");
    let absorbed: Value = serde_json::from_str(&absorbed.to_json().unwrap()).unwrap();
    assert_eq!(absorbed["result"]["status"], "incomplete");
    assert_eq!(
        absorbed["result"]["failure"]["reasons"][0]["kind"],
        "eventRestricted"
    );

    let offline = session
        .analyze_effect("set {_m::*} to all offline players")
        .expect("interface return type must parse as Object");
    let offline: Value = serde_json::from_str(&offline.to_json().unwrap()).unwrap();
    assert_eq!(offline["result"]["status"], "matched");
    assert_eq!(
        offline["result"]["effect"]["elements"][0]["resolved"]["multiplicity"],
        "multiple"
    );
    assert_eq!(
        offline["result"]["effect"]["elements"][1]["resolved"]["returnType"],
        "org.bukkit.OfflinePlayer"
    );
    assert_eq!(
        offline["result"]["effect"]["elements"][1]["resolved"]["multiplicity"],
        "multiple"
    );

    let chat = session
        .analyze_effect("set {_m} to default motd")
        .expect("Component interface return type must parse as Object");
    let chat: Value = serde_json::from_str(&chat.to_json().unwrap()).unwrap();
    assert_eq!(chat["result"]["status"], "matched");
    assert_eq!(
        chat["result"]["effect"]["elements"][1]["resolved"]["returnType"],
        "net.kyori.adventure.text.Component"
    );

    let contextual = session
        .analyze_effect("send player's health to console")
        .expect("missing event context is a normal no-match");
    let contextual: Value = serde_json::from_str(&contextual.to_json().unwrap()).unwrap();
    assert_eq!(contextual["result"]["status"], "incomplete");
    assert_eq!(
        contextual["result"]["effect"]["syntax"]["elementClass"],
        "org.skriptlang.skript.bukkit.text.elements.effects.EffMessage"
    );
    assert!(
        contextual["result"]["failure"]["reasons"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|reason| reason["kind"] == "hookRejected"
                && reason["reason"].as_str().is_some_and(|message| {
                    message == "there is no org.bukkit.entity.Player event value outside an event"
                }))
    );
    assert!(
        contextual["result"]["failure"]["contexts"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|context| context["syntaxKind"] == "Expression")
    );

    let teleport = session
        .analyze_effect("teleport あ to location(1,2,3)")
        .expect("an invalid entity must retain the matching Effect candidate");
    let teleport: Value = serde_json::from_str(&teleport.to_json().unwrap()).unwrap();
    assert_eq!(teleport["result"]["status"], "incomplete");
    assert_eq!(
        teleport["result"]["effect"]["syntax"]["elementClass"],
        "org.skriptlang.skript.bukkit.entity.elements.effects.EffTeleport"
    );
    assert_eq!(teleport["result"]["failure"]["span"]["start"], 9);
    assert!(
        teleport["result"]["failure"]["span"]["end"]
            .as_u64()
            .is_some_and(|end| end > 9)
    );
    assert!(
        teleport["result"]["failure"]["reasons"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|reason| reason["kind"] == "typeExpression"
                && reason["expected"]
                    .as_array()
                    .is_some_and(|expected| expected.iter().any(|value| value == "entity")))
    );
}

#[test]
fn selected_event_context_enables_event_restricted_expressions() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");
    let selected = session
        .select_event_header("\"on join:\"")
        .expect("quoted Event header with a trailing colon must parse")
        .clone();
    assert_eq!(selected.input, "on join");
    assert_eq!(
        selected.reference_events,
        ["org.bukkit.event.player.PlayerJoinEvent"]
    );
    assert!(!selected.event_values.is_empty());

    let report = session
        .analyze_effect("send join message to console")
        .expect("join-only Expression must parse in an On Join context");
    assert!(report.matched());
    assert!(
        session
            .analyze_effect("send event-player's health to console")
            .expect("event-player properties must use the selected Event values")
            .matched()
    );
    assert!(
        session
            .analyze_effect("send player's health to console")
            .expect("ExprEntity must allow Skript's optional event- prefix")
            .matched()
    );
    let interpolated = session
        .analyze_effect("set the player's tab list name to \"<green>%player's name%\"")
        .expect("Event Expressions inside VariableStrings must inherit the selected Event");
    assert!(interpolated.matched());
    let interpolated: Value = serde_json::from_str(&interpolated.to_json().unwrap()).unwrap();
    assert_eq!(interpolated["diagnostics"], serde_json::json!([]));
    let json: Value = serde_json::from_str(&report.to_json().unwrap()).unwrap();
    assert_eq!(json["context"]["event"]["input"], "on join");
    assert_eq!(
        json["context"]["event"]["registrationId"],
        selected.registration_id
    );
    assert_eq!(
        json["context"]["event"]["referenceEvents"][0],
        "org.bukkit.event.player.PlayerJoinEvent"
    );
    assert_eq!(json["context"]["event"]["cancellable"], false);
    assert!(
        json["context"]["event"].get("prioritySupported").is_some(),
        "unresolved priority support must remain an explicit null"
    );
    let event_values = json["context"]["event"]["eventValues"]
        .as_array()
        .expect("selected Event values must be reported");
    let first_event_value = event_values.first().expect("On Join exposes Event values");
    assert!(first_event_value["resolutionOrder"].is_u64());
    assert!(first_event_value["registrationOrder"].is_u64());
    assert!(first_event_value["acceptedChangers"].is_array());
    assert!(first_event_value["patterns"].is_array());
    assert_eq!(first_event_value["addon"]["name"], "Skript");

    let short_header = session
        .select_event_header("join")
        .expect("StructEvent owns the optional on prefix");
    assert_eq!(short_header.registration_id, selected.registration_id);

    let error = session
        .select_event_header("definitely not an event")
        .expect_err("unknown Event must be rejected");
    assert!(
        error
            .to_string()
            .contains("does not match a registered Event")
    );
    assert_eq!(
        session.event_context().unwrap().registration_id,
        selected.registration_id,
        "a rejected selector must not erase the previous Event context"
    );
    assert!(
        session
            .analyze_effect("send join message to console")
            .expect("a rejected selector must not invalidate the previous Event transaction")
            .matched()
    );

    session
        .clear_event_context()
        .expect("the selected Event transaction must close");
    let without_context = session
        .analyze_effect("send join message to console")
        .expect("missing Event context is a recoverable no-match");
    assert!(!without_context.matched());
}

#[test]
fn event_headers_accept_articles_for_entity_and_item_literals() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");

    let death = session
        .select_event_header("death of a player")
        .expect("EntityData must accept Skript's indefinite article");
    assert_eq!(death.pattern, "death [of %-entitydatas%]");
    assert_eq!(
        death.reference_events,
        ["org.bukkit.event.entity.EntityDeathEvent"]
    );

    let click = session
        .select_event_header("rightclick on a sheep holding a diamond sword")
        .expect("EntityData and ItemType aliases must accept indefinite articles");
    assert_eq!(
        click.pattern,
        "[(1:right|2:left)(| |-)][mouse(| |-)]click[ing] [on %-entitydata/itemtype/blockdata%] [(with|using|holding) %-itemtype%]"
    );
    assert!(
        click
            .reference_events
            .iter()
            .any(|event| event == "org.bukkit.event.player.PlayerInteractEvent")
    );
}

#[test]
fn damage_headers_expose_role_constraints_without_inventing_event_values() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");
    let event = session
        .select_event_header("on damage of player by zombie")
        .expect("EvtDamage header literals must parse");

    let captures: serde_json::Value = serde_json::from_str(
        event
            .event_metadata
            .get("parser.event.header-captures")
            .expect("the host must preserve typed Event captures"),
    )
    .expect("typed Event capture metadata must be JSON");
    assert_eq!(captures["captures"][0]["source"], "player");
    assert_eq!(captures["captures"][1]["source"], "zombie");

    let constraints = event
        .structure_metadata
        .iter()
        .find_map(|(key, value)| {
            (key == "event-header-constraints" || key.ends_with("/event-header-constraints"))
                .then_some(value)
        })
        .expect("CoreLibrary must map EvtDamage capture roles");
    let constraints: serde_json::Value =
        serde_json::from_str(constraints).expect("role constraints must be JSON");
    assert_eq!(constraints["constraints"][0]["role"], "event-entity");
    assert_eq!(
        constraints["constraints"][0]["className"],
        "org.bukkit.entity.Player"
    );
    assert_eq!(constraints["constraints"][1]["role"], "damager");

    assert!(
        !session
            .analyze_effect("send player's health to console")
            .expect("an invalid EventValue must be a normal no-match")
            .matched(),
        "Skript requires attacker/victim in damage events; the header filter must not invent `player`"
    );
    assert!(
        session
            .analyze_effect("send victim's health to console")
            .expect("the standard victim Expression must parse")
            .matched()
    );
}

#[test]
fn event_header_modifiers_follow_struct_event_semantics() {
    let mut session = SkriptSession::load(modern_fixture()).expect("fixture must load");

    let error = session
        .select_event_header("cancelled join")
        .expect_err("On Join is not cancellable");
    assert!(error.to_string().contains("cancellation"));
    assert!(session.event_context().is_none());

    let error = session
        .select_event_header("on join with priority monitor:")
        .expect_err("the older fixture does not expose Event priority support");
    let message = error.to_string();
    assert!(message.contains("priorit"), "{message}");

    let selected = session
        .select_event_header("on join:")
        .expect("On Join without optional modifiers must still parse");
    assert!(selected.event_priority.is_none());
    assert_eq!(
        selected.reference_events,
        ["org.bukkit.event.player.PlayerJoinEvent"]
    );
}

#[test]
fn legacy_snapshot_uses_the_synthetic_struct_event_path() {
    let mut session = SkriptSession::load(legacy_fixture()).expect("fixture must load");
    let selected = session
        .select_event_header("on join:")
        .expect("Skript 2.6.4 must expose the legacy Event root through CoreLibrary");
    assert_eq!(
        selected.reference_events,
        ["org.bukkit.event.player.PlayerJoinEvent"]
    );
    let report = session
        .analyze_effect("send join message to console")
        .expect("event-restricted Expressions must use the legacy Event context");
    assert!(report.matched());
}

#[test]
fn section_headers_enable_loop_scoped_effects_and_expressions() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");

    let without_colon = session
        .select_section_header("loop all players")
        .expect("a Section header without its trailing colon must parse")
        .clone();
    assert_eq!(without_colon.input, "loop all players");
    assert!(without_colon.frame.loop_section);

    session
        .clear_section_contexts()
        .expect("the first loop context must clear");
    let with_colon = session
        .select_section_header("loop all players:")
        .expect("a physical Section header with its trailing colon must parse")
        .clone();
    assert_eq!(with_colon.input, "loop all players");
    assert_eq!(
        with_colon.frame.registration_id,
        without_colon.frame.registration_id
    );
    assert_eq!(
        with_colon
            .frame
            .metadata
            .get("nlaocs.core-library/loop-keyed")
            .map(String::as_str),
        Some("false")
    );

    assert!(
        session
            .analyze_effect("continue")
            .expect("continue must inherit the selected loop")
            .matched()
    );
    assert!(
        session
            .analyze_effect("send loop-player to console")
            .expect("loop-value Expressions must inherit the selected loop source")
            .matched()
    );
    let loop_index = session
        .analyze_effect("send loop-index to console")
        .expect("a non-keyed loop index must be a recoverable parse failure");
    assert!(
        !loop_index.matched(),
        "ordinary Expression loops must not expose loop-index:\n{}",
        loop_index.to_json().unwrap()
    );

    session
        .clear_section_contexts()
        .expect("the ordinary loop context must clear");
    let keyed = session
        .select_section_header("loop {values::*}")
        .expect("a list variable loop must parse");
    assert_eq!(
        keyed
            .frame
            .metadata
            .get("nlaocs.core-library/loop-keyed")
            .map(String::as_str),
        Some("true")
    );
    assert!(
        session
            .analyze_effect("send loop-index to console")
            .expect("list variable loops must expose loop-index")
            .matched()
    );
}

#[test]
fn integer_range_keeps_its_long_type_through_shuffle_and_loop_value() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");
    let selected = session
        .select_section_header("loop shuffled (integers between 0 and 8)")
        .expect("ExprNumbers must resolve its mark before SecLoop inspects the source");

    assert_eq!(
        selected
            .frame
            .metadata
            .get("nlaocs.core-library/loop-source-type")
            .map(String::as_str),
        Some("java.lang.Long")
    );
    assert!(
        session
            .analyze_effect("send loop-value to console")
            .expect("loop-value must inherit the resolved integer element type")
            .matched()
    );
}

#[test]
fn legacy_sec_while_provides_loop_control_without_loop_section_flag() {
    let mut session = SkriptSession::load(legacy_fixture()).expect("fixture must load");
    session
        .select_section_header("while 1 is 1")
        .expect("Skript 2.6.4 SecWhile must establish a Section context");

    let report = session
        .analyze_effect("continue")
        .expect("continue must inherit the legacy while loop context");
    assert!(
        report.matched(),
        "continue must match after selecting a 2.6.4 while Section:\n{}",
        report.to_json().unwrap()
    );
}

#[test]
fn nested_section_contexts_restore_on_rejection_pop_and_clear() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");
    session
        .select_section_header("loop all players")
        .expect("outer loop must parse");
    session
        .select_section_header("loop all worlds:")
        .expect("inner loop must parse");

    let sections = session.section_contexts().collect::<Vec<_>>();
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].input, "loop all players");
    assert_eq!(sections[1].input, "loop all worlds");
    assert_eq!(sections[0].frame.parent_scope_id, None);
    assert_eq!(
        sections[1].frame.parent_scope_id,
        Some(sections[0].frame.scope_id)
    );
    let inner_registration = sections[1].frame.registration_id.clone();
    assert!(
        session
            .analyze_effect("exit 2 sections")
            .expect("two active Sections must satisfy EffExit")
            .matched()
    );
    for input in ["exit loop", "exit 2 loops", "exit all loops"] {
        assert!(
            session
                .analyze_effect(input)
                .expect("the loop exit form must parse")
                .matched(),
            "{input}"
        );
    }
    let missing_loop = session
        .analyze_effect("continue 3rd loop")
        .expect("an unavailable loop ordinal is a recoverable incomplete candidate");
    assert!(!missing_loop.matched());
    assert!(
        missing_loop
            .to_json()
            .unwrap()
            .contains("only 2 loop(s) are present")
    );

    let error = session
        .select_section_header("definitely not a section")
        .expect_err("an unknown Section selector must be rejected");
    assert!(
        error
            .to_string()
            .contains("does not match a registered Section")
    );
    let sections = session.section_contexts().collect::<Vec<_>>();
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[1].frame.registration_id, inner_registration);
    assert!(
        session
            .analyze_effect("continue 1st loop")
            .expect("a rejected selector must not damage the retained loop stack")
            .matched()
    );

    let popped = session
        .pop_section_context()
        .expect("the inner loop must pop")
        .expect("an inner loop is active");
    assert_eq!(popped.input, "loop all worlds");
    assert_eq!(session.section_contexts().len(), 1);
    assert_eq!(
        session.section_contexts().next().unwrap().input,
        "loop all players"
    );
    let missing_section = session
        .analyze_effect("exit 2 sections")
        .expect("an unavailable Section depth is a recoverable incomplete candidate");
    assert!(!missing_section.matched());
    assert!(
        missing_section
            .to_json()
            .unwrap()
            .contains("only 1 are present")
    );

    session
        .clear_section_contexts()
        .expect("all remaining Section contexts must clear");
    assert_eq!(session.section_contexts().len(), 0);
    assert!(
        !session
            .analyze_effect("continue")
            .expect("continue without a loop is a recoverable incomplete candidate")
            .matched()
    );
    assert!(
        session
            .analyze_effect("stop trigger")
            .expect("stopping the trigger does not require a Section")
            .matched()
    );
}

#[test]
fn conditional_exit_uses_the_registered_section_frame() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");
    let section = session
        .select_section_header("if true is true:")
        .expect("a SecConditional header must establish a Section context");
    assert_eq!(
        section
            .frame
            .element_class
            .as_ref()
            .map(|class| class.as_str()),
        Some("ch.njol.skript.sections.SecConditional")
    );
    assert!(
        session
            .analyze_effect("exit conditional")
            .expect("EffExit must recognize the conditional frame")
            .matched()
    );

    session.clear_section_contexts().unwrap();
    let report = session
        .analyze_effect("exit conditional")
        .expect("missing conditional context must remain recoverable");
    assert!(!report.matched());
    assert_eq!(
        serde_json::from_str::<Value>(&report.to_json().unwrap()).unwrap()["result"]["status"],
        "incomplete"
    );
}

#[test]
fn json_report_preserves_registered_section_identity() {
    let mut session = SkriptSession::load(type_parser_216_fixture()).expect("fixture must load");
    let selected = session
        .select_section_header("loop all players:")
        .expect("loop Section must parse")
        .clone();
    let report = session
        .analyze_effect("send loop-player to console")
        .expect("loop-player must parse in the selected loop");
    assert!(report.matched());

    let json: Value = serde_json::from_str(&report.to_json().unwrap()).unwrap();
    let sections = json["context"]["sections"]
        .as_array()
        .expect("the report must expose its Section stack");
    assert_eq!(sections.len(), 1);
    let section = &sections[0];
    assert_eq!(section["input"], "loop all players");
    assert_eq!(section["definitionId"], selected.frame.definition_id);
    assert_eq!(section["registrationId"], selected.frame.registration_id);
    assert_eq!(section["elementClass"], "ch.njol.skript.sections.SecLoop");
    assert_eq!(section["kind"], "loopSection");
    assert_eq!(section["pattern"], "loop %objects%");
    assert_eq!(section["addon"]["name"], "Skript");
    assert_eq!(section["addon"]["version"], "2.16.0");
    assert_eq!(section["loopSection"], true);
    assert!(section["scopeId"].is_u64());
    let capture = section["captures"]
        .as_array()
        .expect("section captures must be an array")
        .first()
        .expect("loop Section must expose its first capture");
    assert!(capture["definitionId"].is_string());
    assert!(capture["registrationId"].is_string());
    assert_eq!(
        capture["elementClass"],
        "ch.njol.skript.expressions.ExprEntities"
    );
    assert!(capture["patternIndex"].is_number());
    assert!(capture["publicData"].is_array());
}

#[test]
fn one_shot_and_repl_section_commands_manage_nested_contexts() {
    let snapshot = type_parser_216_fixture();
    let mut output = Vec::new();
    let mut error = Vec::new();
    let code = run_with_io(
        arguments(&[
            "--snapshot",
            snapshot.to_str().unwrap(),
            "--json",
            "--section",
            "loop all players:",
            "--section",
            "loop all worlds",
            "continue 1st loop",
        ]),
        PathBuf::from("unused"),
        Cursor::new(Vec::<u8>::new()),
        &mut output,
        &mut error,
    );
    assert_eq!(code, EXIT_SUCCESS);
    assert!(error.is_empty());
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["result"]["status"], "matched");
    assert_eq!(json["context"]["sections"][0]["input"], "loop all players");
    assert_eq!(json["context"]["sections"][1]["input"], "loop all worlds");

    let input = Cursor::new(
        b":section loop all players:\n:section loop all worlds\n:context\ncontinue 1st loop\n:section pop\n:context\n:section clear\n:context\n:quit\n"
            .to_vec(),
    );
    output.clear();
    error.clear();
    let code = run_with_io(
        arguments(&["--snapshot", snapshot.to_str().unwrap(), "--repl"]),
        PathBuf::from("unused"),
        input,
        &mut output,
        &mut error,
    );
    assert_eq!(code, EXIT_SUCCESS);
    assert!(error.is_empty());
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Section context: loop all players ["));
    assert!(output.contains("Section context: loop all worlds ["));
    assert!(output.contains("Section contexts (outermost to innermost):"));
    assert!(output.contains("1. loop all players ["));
    assert!(output.contains("2. loop all worlds ["));
    assert!(output.contains("EffContinue"));
    assert!(output.contains("Section context popped: loop all worlds"));
    assert!(output.contains("Section contexts cleared"));
    assert!(output.contains("Section contexts: none"));
}

#[test]
fn one_shot_and_repl_event_commands_apply_and_clear_context() {
    let snapshot = modern_fixture();
    let mut output = Vec::new();
    let mut error = Vec::new();
    let code = run_with_io(
        arguments(&[
            "--snapshot",
            snapshot.to_str().unwrap(),
            "--json",
            "--event",
            "on join:",
            "send join message to console",
        ]),
        PathBuf::from("unused"),
        Cursor::new(Vec::<u8>::new()),
        &mut output,
        &mut error,
    );
    assert_eq!(code, EXIT_SUCCESS);
    assert!(error.is_empty());
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["result"]["status"], "matched");
    assert_eq!(json["context"]["event"]["input"], "on join");

    let input = Cursor::new(
        b":events\n:event on join:\n:context\nsend join message to console\n:event off\n:context\n:quit\n"
            .to_vec(),
    );
    output.clear();
    error.clear();
    let code = run_with_io(
        arguments(&["--snapshot", snapshot.to_str().unwrap(), "--repl"]),
        PathBuf::from("unused"),
        input,
        &mut output,
        &mut error,
    );
    assert_eq!(code, EXIT_SUCCESS);
    assert!(error.is_empty());
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Events ("));
    assert!(output.contains("Event context: on join"));
    assert!(output.contains("org.bukkit.event.player.PlayerJoinEvent"));
    assert!(output.contains("ExprJoinMessage"));
    assert!(output.contains("Event context cleared"));
    assert!(output.contains("Event context: none"));
}

#[test]
fn one_shot_json_uses_stable_no_match_exit_code() {
    let snapshot = legacy_fixture();
    let mut output = Vec::new();
    let mut error = Vec::new();
    let code = run_with_io(
        arguments(&[
            "--snapshot",
            snapshot.to_str().unwrap(),
            "--json",
            "__skript_repl_no_match__",
        ]),
        PathBuf::from("unused"),
        Cursor::new(Vec::<u8>::new()),
        &mut output,
        &mut error,
    );
    assert_eq!(code, EXIT_NO_MATCH);
    assert!(error.is_empty());
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["result"]["status"], "unknown");
    assert!(json["result"]["failure"].is_null());
}

#[test]
fn repl_survives_no_match_toggles_json_and_reloads_snapshot() {
    let snapshot = legacy_fixture();
    let input = Cursor::new(
        b"__skript_repl_no_match__\n:json on\nsend 1 to console\n:json off\n:reload\nsend 1 to console\n:quit\n"
            .to_vec(),
    );
    let mut output = Vec::new();
    let mut error = Vec::new();
    let code = run_with_io(
        arguments(&["--snapshot", snapshot.to_str().unwrap(), "--repl"]),
        PathBuf::from("unused"),
        input,
        &mut output,
        &mut error,
    );
    assert_eq!(code, EXIT_SUCCESS);
    assert!(error.is_empty());
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("effect: unknown"));
    assert!(output.contains("JSON output enabled"));
    assert!(output.contains("\"schemaVersion\": 7"));
    assert!(output.contains("JSON output disabled"));
    assert!(output.contains("reloaded"));
    assert!(output.contains("EffMessage"));
}

#[test]
fn manifest_path_is_accepted_by_the_complete_cli() {
    let manifest = legacy_fixture().join("Manifest.json");
    let mut output = Vec::new();
    let mut error = Vec::new();
    let code = run_with_io(
        arguments(&[
            "--snapshot",
            manifest.to_str().unwrap(),
            "send 1 to console",
        ]),
        PathBuf::from("unused"),
        Cursor::new(Vec::<u8>::new()),
        &mut output,
        &mut error,
    );
    assert_eq!(code, EXIT_SUCCESS);
    assert!(error.is_empty());
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("EffMessage"));
    assert!(output.contains("java.lang.Long"));
}

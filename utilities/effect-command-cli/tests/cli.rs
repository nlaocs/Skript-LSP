use effect_command_cli::{
    EXIT_NO_MATCH, EXIT_SUCCESS, EffectCommandSession, OutputFormat, run_with_io,
};
use serde_json::Value;
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

fn arguments(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[test]
fn parses_effect_and_reports_literal_and_type_information() {
    let mut session = EffectCommandSession::load(modern_fixture()).expect("fixture must load");
    let report = session.analyze("send 1").expect("Effect must parse");
    assert!(report.matched());

    let json_text = report.to_json().unwrap();
    assert!(!json_text.contains('\x1b'));
    let json: Value = serde_json::from_str(&json_text).unwrap();
    assert_eq!(json["schemaVersion"], 5);
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
        .expect("send captures one Expression");
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
        .analyze("dummy effect registered through wrapper")
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
    let mut session = EffectCommandSession::load(&snapshot).expect("fixture must load");
    let report = session
        .analyze("send (1)")
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
        arguments(&["--snapshot", snapshot.to_str().unwrap(), "send (1)"]),
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
fn reports_node_local_public_data_as_structured_json() {
    let snapshot = modern_fixture();
    let mut session = EffectCommandSession::load(&snapshot).expect("fixture must load");
    let report = session
        .analyze("send ({_money})")
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
        .analyze("send {_literal%%percent}")
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
            "send {_money}",
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
        arguments(&["--snapshot", snapshot.to_str().unwrap(), "send {_money}"]),
        PathBuf::from("unused"),
        Cursor::new(Vec::<u8>::new()),
        &mut output,
        &mut error,
    );
    assert_eq!(code, EXIT_SUCCESS);
    assert!(error.is_empty());
    let human = String::from_utf8(output).unwrap();
    assert!(human.contains("source: send {_money}"));
    assert!(human.contains("publicData:"));
    assert!(human.contains("schemaId: nlaocs.skript.variable"));
    assert!(human.contains("schemaVersion: 1"));
    assert!(human.contains("json: {"));
    assert!(human.contains("\"money\""));
}

#[test]
fn reports_interpolated_variable_public_data_and_embedded_children() {
    let mut session = EffectCommandSession::load(modern_fixture()).expect("fixture must load");
    let report = session
        .analyze("send {_price::%{_key}%}")
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
    let mut session = EffectCommandSession::load(&snapshot).expect("fixture must load");
    let report = session
        .analyze("send sin(abs(-1))")
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
        arguments(&["--snapshot", snapshot.to_str().unwrap(), "send log(8)"]),
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

    let mut legacy =
        EffectCommandSession::load(legacy_fixture()).expect("legacy fixture must load");
    let legacy: Value = serde_json::from_str(
        &legacy
            .analyze("send sin(1)")
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
    let mut session = EffectCommandSession::load(modern_fixture()).expect("fixture must load");
    let report = session
        .analyze(r#"send "players: %size of all players%""#)
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
    let mut session = EffectCommandSession::load(&snapshot).expect("fixture must load");
    let report = session
        .analyze("return 1 + 2 * 3")
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
    let mut session = EffectCommandSession::load(modern_fixture()).expect("fixture must load");
    for source in ["send 2 if true is true", "send stone"] {
        let report = session
            .analyze(source)
            .expect("Effect analysis must complete");
        assert!(report.matched(), "{source:?} must parse");
    }

    let invalid_comparison = session
        .analyze("send 1 if 1 is true")
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
            "send 2 if true",
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
    assert_eq!(json["result"]["failure"]["span"]["start"], 10);
    assert_eq!(json["result"]["failure"]["span"]["end"], 14);
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
            "send 2 if true",
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
    assert!(output.contains("send 2 if true"));
    assert!(output.contains("unexpected trailing input"));
    assert!(!output.contains('\x1b'));
}

#[test]
fn reports_nested_root_cause_patterns_and_competing_effect_interpretations() {
    let mut session = EffectCommandSession::load(modern_fixture()).expect("fixture must load");
    let report = session
        .analyze("send 1 if a < 5 else 2")
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
        .analyze("send 1 if a < 5 else 2")
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
    let mut session = EffectCommandSession::load(modern_fixture()).expect("fixture must load");

    let absorbed = session
        .analyze("send absorbed blocks")
        .expect("missing Event context must be a normal no-match");
    let absorbed: Value = serde_json::from_str(&absorbed.to_json().unwrap()).unwrap();
    assert_eq!(absorbed["result"]["status"], "incomplete");
    assert_eq!(
        absorbed["result"]["failure"]["reasons"][0]["kind"],
        "eventRestricted"
    );

    let offline = session
        .analyze("set {_m::*} to all offline players")
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
        .analyze("set {_m} to default motd")
        .expect("Component interface return type must parse as Object");
    let chat: Value = serde_json::from_str(&chat.to_json().unwrap()).unwrap();
    assert_eq!(chat["result"]["status"], "matched");
    assert_eq!(
        chat["result"]["effect"]["elements"][1]["resolved"]["returnType"],
        "net.kyori.adventure.text.Component"
    );

    let contextual = session
        .analyze("send player's health")
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
        .analyze("teleport あ to location(1,2,3)")
        .expect("an invalid entity must retain the matching Effect candidate");
    let teleport: Value = serde_json::from_str(&teleport.to_json().unwrap()).unwrap();
    assert_eq!(teleport["result"]["status"], "incomplete");
    assert_eq!(
        teleport["result"]["effect"]["syntax"]["elementClass"],
        "ch.njol.skript.effects.EffTeleport"
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
    let mut session = EffectCommandSession::load(modern_fixture()).expect("fixture must load");
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
        .analyze("send join message")
        .expect("join-only Expression must parse in an On Join context");
    assert!(report.matched());
    assert!(
        session
            .analyze("send event-player's health")
            .expect("event-player properties must use the selected Event values")
            .matched()
    );
    assert!(
        session
            .analyze("send player's health")
            .expect("ExprEntity must allow Skript's optional event- prefix")
            .matched()
    );
    let interpolated = session
        .analyze("set the player's tab list name to \"<green>%player's name%\"")
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
            .analyze("send join message")
            .expect("a rejected selector must not invalidate the previous Event transaction")
            .matched()
    );

    session
        .clear_event_context()
        .expect("the selected Event transaction must close");
    let without_context = session
        .analyze("send join message")
        .expect("missing Event context is a recoverable no-match");
    assert!(!without_context.matched());
}

#[test]
fn event_headers_accept_articles_for_entity_and_item_literals() {
    let mut session = EffectCommandSession::load(modern_fixture()).expect("fixture must load");

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
fn event_header_modifiers_follow_struct_event_semantics() {
    let mut session = EffectCommandSession::load(modern_fixture()).expect("fixture must load");

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
    let mut session = EffectCommandSession::load(legacy_fixture()).expect("fixture must load");
    let selected = session
        .select_event_header("on join:")
        .expect("Skript 2.6.4 must expose the legacy Event root through CoreLibrary");
    assert_eq!(
        selected.reference_events,
        ["org.bukkit.event.player.PlayerJoinEvent"]
    );
    let report = session
        .analyze("send join message")
        .expect("event-restricted Expressions must use the legacy Event context");
    assert!(report.matched());
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
            "send join message",
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
        b":events\n:event on join:\n:context\nsend join message\n:event off\n:context\n:quit\n"
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
            "__effectcommandcli_no_match__",
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
    assert!(json["result"]["failure"]["reasons"].is_array());
}

#[test]
fn repl_survives_no_match_toggles_json_and_reloads_snapshot() {
    let snapshot = legacy_fixture();
    let input = Cursor::new(
        b"__effectcommandcli_no_match__\n:json on\nsend 1\n:json off\n:reload\nsend 1\n:quit\n"
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
    assert!(output.contains("\"schemaVersion\": 5"));
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
        arguments(&["--snapshot", manifest.to_str().unwrap(), "send 1"]),
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

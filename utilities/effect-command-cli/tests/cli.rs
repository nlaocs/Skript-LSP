use effect_command_cli::{EXIT_NO_MATCH, EXIT_SUCCESS, EffectCommandSession, run_with_io};
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

    let json: Value = serde_json::from_str(&report.to_json().unwrap()).unwrap();
    assert_eq!(json["schemaVersion"], 1);
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
    assert!(output.contains("\"schemaVersion\": 1"));
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

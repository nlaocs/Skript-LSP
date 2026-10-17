use serde_json::Value;
use skript_repl::{EXIT_SUCCESS, OutputFormat, SkriptSession, run_with_io};
use std::{
    ffi::OsString,
    io::Cursor,
    path::{Path, PathBuf},
};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/current-multi-addon-2.15.4")
}
fn addon() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/function-addon.wasm")
}

#[test]
fn document_json_preserves_native_and_wasm_function_bindings() {
    let mut session = SkriptSession::load_with_addons(fixture(), [addon()]).unwrap();
    for call in [
        "log(base: 2, n: abs(-8))",
        "wasm named(2;abs(-8))",
        "sum(1, 2)",
        "log(8)",
    ] {
        let source = format!("on load:\n    send {call} to console\n");
        let report = session.analyze_document(&source).unwrap();
        let json: Value = serde_json::from_str(&report.to_json().unwrap()).unwrap();
        assert_eq!(json["schemaVersion"], 2);
        assert_eq!(json["status"], "matched", "{json}");
        let nodes = json["nodes"].as_array().unwrap();
        let functions = nodes
            .iter()
            .filter(|node| node["kind"] == "Function")
            .collect::<Vec<_>>();
        assert!(!functions.is_empty(), "{json}");
        for node in &functions {
            let function = &node["function"];
            assert!(function["definitionId"].as_str().is_some());
            assert!(function["registrationId"].as_str().is_some());
            let children = node["children"].as_array().unwrap();
            let mut next = 0;
            for argument in function["arguments"].as_array().unwrap() {
                assert_eq!(argument["childStart"].as_u64().unwrap(), next);
                let count = argument["childCount"].as_u64().unwrap();
                assert_eq!(argument["omitted"], count == 0);
                next += count;
            }
            assert_eq!(next as usize, children.len());
        }
        let outer = functions
            .iter()
            .find(|node| {
                node["function"]["name"]
                    == if call.starts_with("sum") {
                        "sum"
                    } else {
                        "log"
                    }
            })
            .unwrap();
        let arguments = outer["function"]["arguments"].as_array().unwrap();
        if call.contains("base:") || call.starts_with("wasm named") {
            assert_eq!(arguments[0]["parameterName"], "n");
            assert_eq!(arguments[0]["suppliedName"], "n");
            assert_eq!(arguments[1]["parameterName"], "base");
            assert_eq!(arguments[1]["suppliedName"], "base");
            let first = &outer["children"][0];
            let child = nodes.iter().find(|node| node["id"] == *first).unwrap();
            assert_eq!(child["function"]["name"], "abs");
            assert_eq!(child["text"], "abs(-8)");
        } else if call.starts_with("sum") {
            assert_eq!(arguments[0]["childCount"], 2);
        } else {
            assert_eq!(arguments[1]["omitted"], true);
            assert_eq!(arguments[1]["childCount"], 0);
        }
    }
}

fn assert_function(value: &Value, catalog: &syntaxes::Catalog) {
    if value["expression"]["kind"] != "function" {
        return;
    }
    assert_eq!(value["expression"]["structured"], true);
    let syntax = &value["expression"]["syntax"];
    let definition = catalog
        .syntax_by_registration_id(syntax["registrationId"].as_str().unwrap())
        .into_iter()
        .find_map(|syntax| match syntax {
            syntaxes::Syntax::Function(function) => Some(function),
            _ => None,
        })
        .expect("fixture Function registration");
    assert_eq!(syntax["definitionId"], definition.definition_id.as_str());
    assert_eq!(value["expression"]["name"], definition.name);
    assert_eq!(syntax["addon"]["name"], definition.addon.name);
    assert_eq!(syntax["addon"]["version"], definition.addon.version);
    let arguments = value["arguments"].as_array().unwrap();
    assert_eq!(arguments.len(), definition.parameters.len());
    for (argument, parameter) in arguments.iter().zip(&definition.parameters) {
        assert_eq!(argument["parameterName"], parameter.name);
        for child in argument["values"].as_array().unwrap() {
            assert_function(child, catalog);
        }
    }
}

#[test]
fn wasm_function_identity_arguments_and_recursive_calls_use_the_standard_report() {
    let snapshot = ssg::load_for_parsing(fixture()).unwrap();
    let mut session = SkriptSession::load_with_addons(fixture(), [addon()]).unwrap();
    for (input, name) in [
        (
            r#"send wasm echo(dummy_echo("hello")) to console"#,
            "dummy_echo",
        ),
        (
            r#"send dummy_echo(wasm echo("hello")) to console"#,
            "dummy_echo",
        ),
        (
            r#"send wasm echo(wasm echo("日本語")) to console"#,
            "dummy_echo",
        ),
        ("send wasm log(abs(-8)) to console", "log"),
        ("send wasm named(2;abs(-8)) to console", "log"),
        ("send wasm sum(1;2) to console", "sum"),
        (r#"send wasm attachment("file") to console"#, "attachment"),
    ] {
        let report = session.analyze_effect(input).unwrap();
        assert!(report.matched(), "{input}: {}", report.to_json().unwrap());
        let json: Value = serde_json::from_str(&report.clone().to_json().unwrap()).unwrap();
        let function = &json["result"]["effect"]["elements"][0]["resolved"];
        assert_function(function, snapshot.catalog());
        assert_eq!(function["expression"]["name"], name);
        if input.contains("wasm log") {
            assert_eq!(function["arguments"][1]["omitted"], true);
            assert!(
                function["arguments"][1]["values"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
        if input.contains("wasm named") {
            assert_eq!(function["arguments"][0]["suppliedName"], "n");
            assert_eq!(function["arguments"][0]["values"][0]["source"], "abs(-8)");
            assert_eq!(function["arguments"][1]["suppliedName"], "base");
            assert_eq!(function["arguments"][1]["values"][0]["source"], "2");
        }
        if name == "sum" {
            assert_eq!(
                function["arguments"][0]["values"].as_array().unwrap().len(),
                2
            );
        }
        assert_eq!(
            function["multiplicity"],
            if name == "attachment" {
                "multiple"
            } else {
                "single"
            }
        );
        assert_eq!(
            function["returnType"],
            match name {
                "dummy_echo" => "java.lang.String",
                "attachment" => "cz.coffeerequired.api.requests.Attachment",
                _ => "java.lang.Number",
            }
        );
        let mut human = Vec::new();
        report.write(OutputFormat::Human, &mut human).unwrap();
        let human = String::from_utf8(human).unwrap();
        assert!(human.contains("structured=true"));
        assert!(human.contains(&format!("name: {name}")));
    }
}

#[test]
fn wasm_functions_survive_one_shot_json_repl_and_reload() {
    let snapshot = fixture();
    let addon = addon();
    let base = [
        "--snapshot",
        snapshot.to_str().unwrap(),
        "--addon",
        addon.to_str().unwrap(),
        "--no-color",
    ];
    for (extra, input) in [
        (vec!["--json", r#"send wasm echo(dummy_echo("hello")) to console"#], String::new()),
        (vec!["--repl"], ":json on\nsend wasm log(abs(-8)) to console\n:json off\n:reload\nsend wasm echo(\"日本語\") to console\n:quit\n".into()),
    ] {
        let arguments = base.iter().copied().chain(extra).map(OsString::from).collect::<Vec<_>>();
        let mut out = Vec::new(); let mut err = Vec::new();
        assert_eq!(run_with_io(arguments, PathBuf::from("unused"), Cursor::new(input.into_bytes()), &mut out, &mut err), EXIT_SUCCESS);
        assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("test.function-addon.call"));
        assert!(out.contains("dummy_echo"));
        assert!(!out.contains('\u{1b}'));
        if out.contains("reloaded") { assert!(out.contains("structured=true")); }
        else { let json: Value = serde_json::from_str(&out).unwrap(); assert_eq!(json["result"]["effect"]["elements"][0]["resolved"]["expression"]["structured"], true); }
    }
}

#[test]
fn malformed_wasm_function_mappings_are_faults_and_preserve_a_later_line() {
    for mode in [
        "range",
        "overlap",
        "unowned",
        "omitted",
        "name",
        "missing",
        "kind",
        "span",
        "reference",
    ] {
        let mut session = SkriptSession::load_with_addons(fixture(), [addon()]).unwrap();
        let source = format!(
            "on load:\n    send wasm bad-{mode}(\"x\") to console\n    send 2 to console\n"
        );
        let report = session.analyze_document(&source).unwrap();
        let json: Value = serde_json::from_str(&report.to_json().unwrap()).unwrap();
        assert_eq!(json["status"], "incomplete", "{mode}: {json}");
        assert!(
            json["faultRegions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|fault| fault["componentId"] == "test.function-addon"
                    && fault["code"] == "parser.extension-invalid-output"),
            "{mode}: {json}"
        );
        assert!(
            json["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|node| node["kind"] == "Effect" && node["text"] == "send 2 to console")
        );
        assert!(
            session
                .analyze_effect("send 3 to console")
                .unwrap()
                .matched()
        );
    }
}

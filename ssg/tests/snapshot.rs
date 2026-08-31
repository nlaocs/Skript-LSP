use sha2::{Digest, Sha256};
use ssg::{SnapshotError, load};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use syntaxes::{ResolutionState, Syntax};

fn modern_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn legacy_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/legacy-2.6.4-mc-1.12.2")
}

#[test]
fn loads_and_indexes_modern_multi_addon_snapshot() {
    let snapshot = load(modern_fixture()).expect("modern schema 3 fixture must load");
    let manifest = snapshot.manifest();
    let catalog = snapshot.catalog();

    assert_eq!(manifest.schema_version, 3);
    assert_eq!(manifest.server.minecraft_version, "1.21.11");
    assert_eq!(manifest.plugins.len(), 9);
    assert!(manifest.capabilities.syntax_kinds.structures);
    assert!(manifest.capabilities.aliases.collected);

    assert_eq!(catalog.events().count(), 316);
    assert_eq!(catalog.conditions().count(), 297);
    assert_eq!(catalog.effects().count(), 285);
    assert_eq!(catalog.expressions().count(), 1043);
    assert_eq!(catalog.types().count(), 257);
    assert_eq!(catalog.functions().count(), 63);
    assert_eq!(catalog.sections().count(), 86);
    assert_eq!(catalog.structures().count(), 22);
    assert_eq!(catalog.syntaxes().len(), 2369);

    assert!(matches!(catalog.syntaxes().first(), Some(Syntax::Event(_))));
    assert!(matches!(
        catalog.syntaxes().get(316),
        Some(Syntax::Condition(_))
    ));

    let player = catalog
        .type_by_code_name("player")
        .expect("player type must be indexed");
    assert_eq!(player.original_class.as_str(), "org.bukkit.entity.Player");
    assert!(catalog.is_type_assignable("player", "entity"));
    assert!(catalog.is_class_assignable("org.bukkit.entity.Player", "org.bukkit.entity.Entity"));

    let uuid = catalog.functions_named("uuid");
    assert_eq!(uuid.len(), 1);
    assert_eq!(uuid[0].name, "uuid");
    assert_eq!(uuid[0].parameters.len(), 1);

    assert!(
        catalog
            .converters_from("java.lang.Number")
            .iter()
            .any(|converter| converter.to.as_str() == "java.lang.Integer")
    );
    assert!(
        catalog
            .event_values_for("org.bukkit.event.player.PlayerJoinEvent")
            .iter()
            .any(|value| value.value_class.as_str() == "org.bukkit.entity.Player")
    );
    assert!(catalog.alias("stone").is_some());

    let material = catalog
        .type_by_code_name("material")
        .expect("material type must be indexed");
    assert!(material.usage.is_empty());
    assert!(material.enum_values.iter().any(|value| value == "stone"));
    assert!(!material.has_parser);
    assert!(
        catalog
            .type_literals("stone")
            .all(|value| value.code_name.as_str() != "material")
    );

    let first_plural_rule = &catalog.plural_rules().rules()[0];
    assert_eq!(first_plural_rule.addon().name(), "SkriptDummyAddon");
}

#[test]
fn loads_schema_four_fixture_without_language_file() {
    let directory = materialize_snapshot(4, None);
    let snapshot = load(directory.path()).expect("schema 4 fixture must load");

    assert_eq!(snapshot.manifest().schema_version, 4);
    assert!(snapshot.catalog().language_entries().next().is_none());

    let source = snapshot
        .catalog()
        .source()
        .expect("SSG-loaded catalogs must retain their source view");
    assert_eq!(
        source.document_names().collect::<Vec<_>>(),
        ssg::LEGACY_ALL_FILES.to_vec()
    );
    assert_eq!(source.document("Language.json"), None);
}

#[test]
fn schema_five_loads_language_and_exposes_current_inventory() {
    let language = serde_json::json!({
        "message.empty": "",
        "message.send": "Send",
        "message.teleport": "Teleport"
    });
    let directory = materialize_snapshot(5, Some(language));
    let snapshot = load(directory.path()).expect("schema 5 fixture must load");

    assert_eq!(ssg::DATA_FILES.len(), 19);
    assert_eq!(ssg::ALL_FILES.len(), 20);
    assert_eq!(ssg::data_files_for_schema(3).unwrap().len(), 18);
    assert_eq!(ssg::all_files_for_schema(4).unwrap().len(), 19);
    assert_eq!(ssg::data_files_for_schema(5).unwrap(), ssg::DATA_FILES);
    assert_eq!(ssg::all_files_for_schema(5).unwrap(), ssg::ALL_FILES);

    let expected_files = ssg::ALL_FILES
        .iter()
        .map(|file| (*file).to_owned())
        .collect::<Vec<_>>();
    assert_eq!(snapshot.manifest().files, expected_files);

    let catalog = snapshot.catalog();
    assert_eq!(catalog.language_value("message.send"), Some("Send"));
    assert_eq!(catalog.language_value("message.empty"), Some(""));
    assert_eq!(catalog.language_value("missing.key"), None);
    assert_eq!(
        catalog.language_entries().collect::<Vec<_>>(),
        vec![
            ("message.empty", ""),
            ("message.send", "Send"),
            ("message.teleport", "Teleport")
        ]
    );

    let language_bytes = fs::read(directory.path().join("Language.json")).unwrap();
    let source = catalog
        .source()
        .expect("SSG-loaded catalogs must retain their source view");
    assert_eq!(
        source.document_names().collect::<Vec<_>>(),
        ssg::ALL_FILES.to_vec()
    );
    assert_eq!(
        source.document("Language.json"),
        Some(language_bytes.as_slice())
    );
}

#[test]
fn schema_five_requires_language_file() {
    let directory = materialize_snapshot(5, None);
    fs::remove_file(directory.path().join("Language.json")).unwrap();

    assert!(matches!(
        load(directory.path()).unwrap_err(),
        SnapshotError::MissingFile {
            file: "Language.json"
        }
    ));
}

#[test]
fn schema_five_requires_language_object_with_string_values() {
    let non_object = materialize_snapshot(5, Some(serde_json::json!([])));
    assert!(matches!(
        load(non_object.path()).unwrap_err(),
        SnapshotError::Validation { path, message }
            if path == "Language.json" && message == "language root must be an object"
    ));

    let non_string = materialize_snapshot(5, Some(serde_json::json!({"message.send": true})));
    assert!(matches!(
        load(non_string.path()).unwrap_err(),
        SnapshotError::Validation { path, message }
            if path == "Language.json.message.send" && message == "language values must be strings"
    ));
}

#[test]
fn loads_legacy_264_on_minecraft_1122() {
    let snapshot = load(legacy_fixture()).expect("legacy schema 3 fixture must load");
    let manifest = snapshot.manifest();
    let catalog = snapshot.catalog();

    assert_eq!(manifest.server.minecraft_version, "1.12.2");
    assert_eq!(manifest.plugins[1].version, "2.6.4");
    assert_eq!(
        manifest.capabilities.syntax_api,
        ssg::raw::SyntaxApi::LegacyStatic
    );
    assert_eq!(
        manifest.capabilities.event_value_api,
        ssg::raw::EventValueApi::Legacy
    );
    assert!(!manifest.capabilities.syntax_kinds.structures);
    assert!(!manifest.capabilities.syntax_kinds.properties);
    assert!(!manifest.capabilities.syntax_kinds.arithmetic);

    assert_eq!(catalog.events().count(), 125);
    assert_eq!(catalog.conditions().count(), 66);
    assert_eq!(catalog.effects().count(), 73);
    assert_eq!(catalog.expressions().count(), 260);
    assert_eq!(catalog.types().count(), 65);
    assert_eq!(catalog.functions().count(), 29);
    assert_eq!(catalog.sections().count(), 5);
    assert_eq!(catalog.structures().count(), 0);
    assert!(catalog.properties().is_empty());
    assert!(catalog.operators().is_empty());
    assert!(catalog.operations().is_empty());
    assert!(catalog.differences().is_empty());
    assert!(catalog.alias("stone").is_some());
    assert!(catalog.expressions().all(|expression| {
        expression.return_type_multiplicity_state == ResolutionState::Unresolved
            && expression.accepted_changers_state == ResolutionState::Unresolved
    }));
}

#[test]
fn into_catalog_exposes_the_complete_source_view() {
    let snapshot = load(modern_fixture()).expect("modern schema 3 fixture must load");
    let expected_snapshot_id = snapshot.manifest().snapshot_id.clone();
    let catalog = snapshot.into_catalog();
    let source = catalog
        .source()
        .expect("SSG-loaded catalogs must retain their source view");

    assert_eq!(source.format, "ssg");
    assert_eq!(source.schema_version, 3);
    assert_eq!(source.snapshot_id, expected_snapshot_id);
    assert_eq!(
        source.document_names().collect::<Vec<_>>(),
        ssg::LEGACY_ALL_FILES.to_vec()
    );

    let manifest = fs::read(modern_fixture().join("Manifest.json")).unwrap();
    let effects = fs::read(modern_fixture().join("Effects.json")).unwrap();
    assert_eq!(source.document("Manifest.json"), Some(manifest.as_slice()));
    assert_eq!(source.document("Effects.json"), Some(effects.as_slice()));

    let effect = catalog
        .syntaxes()
        .iter()
        .find(|syntax| matches!(syntax, Syntax::Effect(_)))
        .expect("modern fixture must contain an effect");
    let registration_id = effect.registration_id().as_str().to_owned();
    let definition_id = effect.definition_id().as_str().to_owned();
    assert!(
        source
            .records_by_registration_id(&registration_id)
            .iter()
            .any(|record| record.document == "Effects.json")
    );
    assert!(
        source
            .records_by_definition_id(&definition_id)
            .iter()
            .any(|record| record.document == "Effects.json")
    );
}

#[test]
fn rejects_unsupported_schema_before_reading_data_files() {
    let directory = tempfile::tempdir().unwrap();
    let mut manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(modern_fixture().join("Manifest.json")).unwrap())
            .unwrap();
    manifest["schemaVersion"] = 6.into();
    fs::write(
        directory.path().join("Manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();

    let error = load(directory.path()).unwrap_err();
    assert!(matches!(
        error,
        SnapshotError::UnsupportedSchema {
            minimum: 3,
            maximum: 5,
            actual: 6
        }
    ));
}

#[test]
fn reports_manifest_json_paths() {
    let directory = tempfile::tempdir().unwrap();
    let mut manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(modern_fixture().join("Manifest.json")).unwrap())
            .unwrap();
    manifest["server"].as_object_mut().unwrap().remove("name");
    fs::write(
        directory.path().join("Manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();

    let error = load(directory.path()).unwrap_err();
    match error {
        SnapshotError::Json { file, path, .. } => {
            assert_eq!(file, "Manifest.json");
            assert!(path.contains("server"), "unexpected JSON path: {path}");
        }
        other => panic!("expected JSON error, found {other:?}"),
    }
}

#[test]
fn rejects_missing_and_tampered_files() {
    let missing = tempfile::tempdir().unwrap();
    fs::copy(
        modern_fixture().join("Manifest.json"),
        missing.path().join("Manifest.json"),
    )
    .unwrap();
    assert!(matches!(
        load(missing.path()).unwrap_err(),
        SnapshotError::MissingFile {
            file: "Aliases.json"
        }
    ));

    let tampered = tempfile::tempdir().unwrap();
    copy_snapshot(&modern_fixture(), tampered.path());
    let types = tampered.path().join("Types.json");
    let mut text = fs::read_to_string(&types).unwrap();
    text.push(' ');
    fs::write(types, text).unwrap();
    assert!(matches!(
        load(tampered.path()).unwrap_err(),
        SnapshotError::ContentDigest { .. }
    ));
}

#[test]
fn ignores_unknown_manifest_fields() {
    let directory = tempfile::tempdir().unwrap();
    copy_snapshot(&modern_fixture(), directory.path());
    let manifest_path = directory.path().join("Manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
    manifest.as_object_mut().unwrap().insert(
        "futureField".to_owned(),
        serde_json::json!({ "value": true }),
    );
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    load(directory.path()).expect("unknown fields must not break schema 3 readers");
}

fn copy_snapshot(source: &Path, target: &Path) {
    for file in ssg::LEGACY_ALL_FILES {
        fs::copy(source.join(file), target.join(file)).unwrap();
    }
}

fn materialize_snapshot(
    schema_version: u32,
    language: Option<serde_json::Value>,
) -> tempfile::TempDir {
    assert!((4..=5).contains(&schema_version));

    let directory = tempfile::tempdir().unwrap();
    copy_snapshot(&modern_fixture(), directory.path());

    if schema_version >= 4 {
        let path = directory.path().join("Expressions.json");
        let mut expressions: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        for expression in expressions.as_array_mut().unwrap() {
            let has_return_type = expression
                .get("returnType")
                .is_some_and(|value| !value.is_null());
            expression["returnTypeState"] = serde_json::json!(if has_return_type {
                "static"
            } else {
                "unresolved"
            });

            let has_possible_return_types = expression
                .get("possibleReturnTypes")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|values| !values.is_empty());
            expression["possibleReturnTypesState"] =
                serde_json::json!(if has_possible_return_types {
                    "partial"
                } else {
                    "unresolved"
                });
        }
        fs::write(&path, serde_json::to_vec(&expressions).unwrap()).unwrap();
    }

    if schema_version >= 5 {
        let path = directory.path().join("ClassHierarchy.json");
        let mut classes: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        for class in classes.as_array_mut().unwrap() {
            class["methods"] = serde_json::json!([]);
        }
        fs::write(&path, serde_json::to_vec(&classes).unwrap()).unwrap();

        let language = language.unwrap_or_else(|| serde_json::json!({}));
        fs::write(
            directory.path().join("Language.json"),
            serde_json::to_vec(&language).unwrap(),
        )
        .unwrap();
    }

    let manifest_path = directory.path().join("Manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
    manifest["schemaVersion"] = schema_version.into();

    let files = ssg::all_files_for_schema(schema_version).unwrap();
    manifest["files"] = serde_json::Value::Array(
        files
            .iter()
            .map(|file| serde_json::Value::String((*file).to_owned()))
            .collect(),
    );

    let data_files = ssg::data_files_for_schema(schema_version).unwrap();
    let serialized = data_files
        .iter()
        .map(|file| {
            (
                (*file).to_owned(),
                fs::read_to_string(directory.path().join(file)).unwrap(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    manifest["contentDigest"] = serde_json::Value::String(content_digest(&serialized));
    manifest["snapshotId"] = serde_json::Value::String(snapshot_id(&manifest));
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    directory
}

fn content_digest(files: &BTreeMap<String, String>) -> String {
    let mut digest = Sha256::new();
    for (index, (file_name, json)) in files.iter().enumerate() {
        if index > 0 {
            digest.update(b"|");
        }
        append_sized(&mut digest, file_name);
        append_sized(&mut digest, json);
    }
    hex_digest(digest.finalize())
}

fn snapshot_id(manifest: &serde_json::Value) -> String {
    let plugin_fingerprints = manifest["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .map(plugin_fingerprint)
        .collect::<Vec<_>>();
    let mut files = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    files.sort();

    let encoded = fingerprint(&[
        manifest["schemaVersion"].as_u64().unwrap().to_string(),
        manifest["contentDigest"].as_str().unwrap().to_owned(),
        server_fingerprint(&manifest["server"]),
        manifest["language"].as_str().unwrap().to_owned(),
        fingerprint(&plugin_fingerprints),
        capabilities_fingerprint(&manifest["capabilities"]),
        fingerprint(&files),
    ]);
    hex_digest(Sha256::digest(encoded.as_bytes()))
}

fn server_fingerprint(server: &serde_json::Value) -> String {
    fingerprint(&[
        server["name"].as_str().unwrap().to_owned(),
        server["version"].as_str().unwrap().to_owned(),
        server["bukkitVersion"].as_str().unwrap().to_owned(),
        server["minecraftVersion"].as_str().unwrap().to_owned(),
        server["javaVersion"].as_str().unwrap().to_owned(),
    ])
}

fn plugin_fingerprint(plugin: &serde_json::Value) -> String {
    fingerprint(&[
        plugin["loadOrder"].as_u64().unwrap().to_string(),
        plugin["name"].as_str().unwrap().to_owned(),
        plugin["version"].as_str().unwrap().to_owned(),
        plugin["main"].as_str().unwrap().to_owned(),
        plugin["enabled"].as_bool().unwrap().to_string(),
        joined_array(&plugin["depend"]),
        joined_array(&plugin["softDepend"]),
        joined_array(&plugin["loadBefore"]),
        plugin["jarSha256"].as_str().unwrap_or_default().to_owned(),
    ])
}

fn capabilities_fingerprint(capabilities: &serde_json::Value) -> String {
    let kinds = &capabilities["syntaxKinds"];
    let kind_bits = [
        "conditions",
        "effects",
        "events",
        "expressions",
        "types",
        "functions",
        "sections",
        "structures",
        "properties",
        "arithmetic",
        "converters",
        "comparators",
        "eventValues",
    ]
    .iter()
    .map(|field| {
        if kinds[*field].as_bool().unwrap() {
            '1'
        } else {
            '0'
        }
    })
    .collect::<String>();
    let aliases = format!(
        "{}:{}",
        u8::from(capabilities["aliases"]["supported"].as_bool().unwrap()),
        u8::from(capabilities["aliases"]["collected"].as_bool().unwrap())
    );

    fingerprint(&[
        capabilities["syntaxApi"].as_str().unwrap().to_owned(),
        capabilities["eventValueApi"].as_str().unwrap().to_owned(),
        kind_bits,
        aliases,
    ])
}

fn joined_array(values: &serde_json::Value) -> String {
    values
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>()
        .join(",")
}

fn fingerprint(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| format!("{}:{part}", java_len(part)))
        .collect::<Vec<_>>()
        .join("|")
}

fn append_sized(digest: &mut Sha256, value: &str) {
    digest.update(java_len(value).to_string().as_bytes());
    digest.update(b":");
    digest.update(value.as_bytes());
}

fn java_len(value: &str) -> usize {
    value.encode_utf16().count()
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

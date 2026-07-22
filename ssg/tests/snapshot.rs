use ssg::{SnapshotError, load};
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

    let first_plural_rule = &catalog.plural_rules().rules()[0];
    assert_eq!(first_plural_rule.addon().name(), "SkriptDummyAddon");
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
fn rejects_unsupported_schema_before_reading_data_files() {
    let directory = tempfile::tempdir().unwrap();
    let mut manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(modern_fixture().join("Manifest.json")).unwrap())
            .unwrap();
    manifest["schemaVersion"] = 4.into();
    fs::write(
        directory.path().join("Manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();

    let error = load(directory.path()).unwrap_err();
    assert!(matches!(
        error,
        SnapshotError::UnsupportedSchema {
            expected: 3,
            actual: 4
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
    for file in ssg::ALL_FILES {
        fs::copy(source.join(file), target.join(file)).unwrap();
    }
}

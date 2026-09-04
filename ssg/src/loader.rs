//! Snapshot directory I/O and the public loading pipeline.
//!
//! This module gates the schema before reading data, verifies both digests, then
//! deserializes, validates, and converts the complete snapshot atomically.

use crate::{SnapshotError, convert, digest, raw, validate};
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use syntaxes::{Catalog, CatalogSource};

/// Highest SSG snapshot schema accepted by this reader.
pub const SCHEMA_VERSION: u32 = 5;
/// Oldest SSG snapshot schema accepted by this reader.
pub const MIN_SCHEMA_VERSION: u32 = 3;
/// Canonical manifest filename.
pub const MANIFEST_FILE: &str = "Manifest.json";

/// Data files covered by schema 3 and 4 content digests, in canonical order.
pub const LEGACY_DATA_FILES: [&str; 18] = [
    "Aliases.json",
    "ClassHierarchy.json",
    "Comparators.json",
    "Conditions.json",
    "Converters.json",
    "Differences.json",
    "Effects.json",
    "EventValues.json",
    "Events.json",
    "Expressions.json",
    "Functions.json",
    "Operations.json",
    "Operators.json",
    "PluralRules.json",
    "Properties.json",
    "Sections.json",
    "Structures.json",
    "Types.json",
];

/// Complete schema 3 and 4 snapshot inventory, including `Manifest.json`.
pub const LEGACY_ALL_FILES: [&str; 19] = [
    "Aliases.json",
    "ClassHierarchy.json",
    "Comparators.json",
    "Conditions.json",
    "Converters.json",
    "Differences.json",
    "Effects.json",
    "EventValues.json",
    "Events.json",
    "Expressions.json",
    "Functions.json",
    "Manifest.json",
    "Operations.json",
    "Operators.json",
    "PluralRules.json",
    "Properties.json",
    "Sections.json",
    "Structures.json",
    "Types.json",
];

/// Data files covered by the current schema 5 content digest, in canonical order.
pub const DATA_FILES: [&str; 19] = [
    "Aliases.json",
    "ClassHierarchy.json",
    "Comparators.json",
    "Conditions.json",
    "Converters.json",
    "Differences.json",
    "Effects.json",
    "EventValues.json",
    "Events.json",
    "Expressions.json",
    "Functions.json",
    "Language.json",
    "Operations.json",
    "Operators.json",
    "PluralRules.json",
    "Properties.json",
    "Sections.json",
    "Structures.json",
    "Types.json",
];

/// Complete current schema 5 snapshot inventory, including `Manifest.json`.
pub const ALL_FILES: [&str; 20] = [
    "Aliases.json",
    "ClassHierarchy.json",
    "Comparators.json",
    "Conditions.json",
    "Converters.json",
    "Differences.json",
    "Effects.json",
    "EventValues.json",
    "Events.json",
    "Expressions.json",
    "Functions.json",
    "Language.json",
    "Manifest.json",
    "Operations.json",
    "Operators.json",
    "PluralRules.json",
    "Properties.json",
    "Sections.json",
    "Structures.json",
    "Types.json",
];

/// Returns the data-file inventory required by a supported schema version.
pub fn data_files_for_schema(schema_version: u32) -> Option<&'static [&'static str]> {
    match schema_version {
        3 | 4 => Some(&LEGACY_DATA_FILES),
        5 => Some(&DATA_FILES),
        _ => None,
    }
}

/// Returns the complete file inventory required by a supported schema version.
pub fn all_files_for_schema(schema_version: u32) -> Option<&'static [&'static str]> {
    match schema_version {
        3 | 4 => Some(&LEGACY_ALL_FILES),
        5 => Some(&ALL_FILES),
        _ => None,
    }
}

#[derive(Debug, Clone)]
/// Fully verified snapshot containing its source manifest and runtime catalog.
///
/// A snapshot keeps provenance and runtime data together: [`Snapshot::manifest`]
/// identifies the exact server/plugin environment, while [`Snapshot::catalog`]
/// exposes normalized indexes suitable for parser and LSP queries.
///
/// # Examples
///
/// ~~~no_run
/// use ssg::load;
///
/// let snapshot = load("run/plugins/SkriptSyntaxGenerator")?;
/// let manifest = snapshot.manifest();
///
/// println!(
///     "Minecraft {} with {} plugins: {} syntaxes",
///     manifest.server.minecraft_version,
///     manifest.plugins.len(),
///     snapshot.catalog().syntaxes().len(),
/// );
///
/// // Catalog queries preserve the registration order captured by SSG.
/// for expression in snapshot.catalog().expressions().take(3) {
///     println!("{}", expression.common.registration_id.as_str());
/// }
/// # Ok::<(), ssg::SnapshotError>(())
/// ~~~
pub struct Snapshot {
    manifest: raw::Manifest,
    catalog: Catalog,
}

impl Snapshot {
    /// Returns the raw manifest used to identify the server and capabilities.
    pub fn manifest(&self) -> &raw::Manifest {
        &self.manifest
    }

    /// Returns the normalized immutable syntax catalog.
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Consumes the snapshot and returns its normalized catalog.
    pub fn into_catalog(self) -> Catalog {
        self.catalog
    }
}

/// Loads and atomically validates one complete supported SSG snapshot directory.
///
/// Unknown JSON fields are accepted, but required files, hashes, identities,
/// resolution states, references, and syntax patterns are verified before a
/// [`Snapshot`] is returned.
///
/// Loading is all-or-nothing. A successful return means every listed file was
/// read, both manifest digests matched, cross-file references were valid, and
/// every syntax pattern was parsed with the snapshot's plural rules.
///
/// # Examples
///
/// ~~~no_run
/// use ssg::{MIN_SCHEMA_VERSION, SCHEMA_VERSION, load};
///
/// let snapshot = load("path/to/generated-snapshot")?;
/// assert!((MIN_SCHEMA_VERSION..=SCHEMA_VERSION).contains(&snapshot.manifest().schema_version));
///
/// let catalog = snapshot.catalog();
/// if let Some(string_type) = catalog.type_by_code_name("string") {
///     println!("string is represented by {}", string_type.original_class.as_str());
/// }
/// # Ok::<(), ssg::SnapshotError>(())
/// ~~~
///
/// # Errors
///
/// Returns [`SnapshotError`] with file and JSON-path context for I/O, format,
/// integrity, semantic validation, or pattern failures.
pub fn load(directory: impl AsRef<Path>) -> Result<Snapshot, SnapshotError> {
    let directory = directory.as_ref();
    let manifest_text = read_file(directory, MANIFEST_FILE)?;
    let manifest: raw::Manifest = parse_json(MANIFEST_FILE, &manifest_text)?;

    if !(MIN_SCHEMA_VERSION..=SCHEMA_VERSION).contains(&manifest.schema_version) {
        return Err(SnapshotError::UnsupportedSchema {
            minimum: MIN_SCHEMA_VERSION,
            maximum: SCHEMA_VERSION,
            actual: manifest.schema_version,
        });
    }
    let data_files = data_files_for_schema(manifest.schema_version)
        .expect("schema range was checked immediately above");
    let all_files = all_files_for_schema(manifest.schema_version)
        .expect("schema range was checked immediately above");
    validate::manifest(&manifest, all_files)?;

    let mut serialized = BTreeMap::new();
    for file in data_files.iter().copied() {
        serialized.insert(file, read_file(directory, file)?);
    }

    let actual_digest = digest::content_digest(&serialized);
    if actual_digest != manifest.content_digest {
        return Err(SnapshotError::ContentDigest {
            expected: manifest.content_digest.clone(),
            actual: actual_digest,
        });
    }

    let actual_snapshot_id = digest::snapshot_id(&manifest);
    if actual_snapshot_id != manifest.snapshot_id {
        return Err(SnapshotError::SnapshotId {
            expected: manifest.snapshot_id.clone(),
            actual: actual_snapshot_id,
        });
    }

    let raw_snapshot = raw::Snapshot {
        aliases: parse(&serialized, "Aliases.json")?,
        classes: parse(&serialized, "ClassHierarchy.json")?,
        comparators: parse(&serialized, "Comparators.json")?,
        conditions: parse(&serialized, "Conditions.json")?,
        converters: parse(&serialized, "Converters.json")?,
        differences: parse(&serialized, "Differences.json")?,
        effects: parse(&serialized, "Effects.json")?,
        event_values: parse(&serialized, "EventValues.json")?,
        events: parse(&serialized, "Events.json")?,
        expressions: parse(&serialized, "Expressions.json")?,
        functions: parse(&serialized, "Functions.json")?,
        language: if manifest.schema_version >= 5 {
            parse_language(&serialized, "Language.json")?
        } else {
            BTreeMap::new()
        },
        operations: parse(&serialized, "Operations.json")?,
        operators: parse(&serialized, "Operators.json")?,
        plural_rules: parse(&serialized, "PluralRules.json")?,
        properties: parse(&serialized, "Properties.json")?,
        sections: parse(&serialized, "Sections.json")?,
        structures: parse(&serialized, "Structures.json")?,
        types: parse(&serialized, "Types.json")?,
    };

    validate::snapshot(&manifest, &raw_snapshot)?;
    let plural_rules =
        syntax_pattern_parser::syntax::PluralRules::from_json(&serialized["PluralRules.json"])
            .map_err(|error| SnapshotError::validation("PluralRules.json", error.to_string()))?;
    let source_documents = std::iter::once((MANIFEST_FILE.to_owned(), manifest_text.into_bytes()))
        .chain(
            serialized
                .into_iter()
                .map(|(name, text)| (name.to_owned(), text.into_bytes())),
        )
        .collect::<BTreeMap<_, _>>();
    let source = CatalogSource::from_json_documents(
        "ssg",
        manifest.schema_version,
        manifest.snapshot_id.clone(),
        source_documents,
    )
    .map_err(|error| SnapshotError::validation("snapshot source", error.to_string()))?
    .with_runtime(syntaxes::CatalogRuntime {
        server_name: manifest.server.name.clone(),
        server_version: manifest.server.version.clone(),
        minecraft_version: manifest.server.minecraft_version.clone(),
        java_version: manifest.server.java_version.clone(),
        language: manifest.language.clone(),
        plugins: manifest
            .plugins
            .iter()
            .map(|plugin| syntaxes::CatalogRuntimePlugin {
                load_order: plugin.load_order,
                name: plugin.name.clone(),
                version: plugin.version.clone(),
                main: plugin.main.clone(),
                enabled: plugin.enabled,
            })
            .collect(),
    });
    // Both views were built from the same validated `serialized` document set above.
    let catalog = convert::catalog(raw_snapshot, plural_rules)?.with_unchecked_source(source);

    Ok(Snapshot { manifest, catalog })
}

fn parse_language(
    serialized: &BTreeMap<&'static str, String>,
    file: &'static str,
) -> Result<BTreeMap<String, String>, SnapshotError> {
    let value: serde_json::Value = parse_json(file, &serialized[file])?;
    let Some(object) = value.as_object() else {
        return Err(SnapshotError::validation(
            file,
            "language root must be an object",
        ));
    };

    object
        .iter()
        .map(|(key, value)| {
            let Some(value) = value.as_str() else {
                return Err(SnapshotError::validation(
                    format!("{file}.{key}"),
                    "language values must be strings",
                ));
            };
            Ok((key.clone(), value.to_owned()))
        })
        .collect()
}

fn parse<T: DeserializeOwned>(
    serialized: &BTreeMap<&'static str, String>,
    file: &'static str,
) -> Result<T, SnapshotError> {
    parse_json(file, &serialized[file])
}

fn parse_json<T: DeserializeOwned>(file: &'static str, text: &str) -> Result<T, SnapshotError> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    serde_path_to_error::deserialize(&mut deserializer).map_err(|error| SnapshotError::Json {
        file,
        path: error.path().to_string(),
        source: error.into_inner(),
    })
}

fn read_file(directory: &Path, file: &'static str) -> Result<String, SnapshotError> {
    let path: PathBuf = directory.join(file);
    fs::read_to_string(&path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            SnapshotError::MissingFile { file }
        } else {
            SnapshotError::Io { path, source }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_nested_paths_for_data_file_json_errors() {
        let error = parse_json::<Vec<raw::Function>>(
            "Functions.json",
            r#"[{"registrationOrder":"not-a-number"}]"#,
        )
        .unwrap_err();

        match error {
            SnapshotError::Json { file, path, .. } => {
                assert_eq!(file, "Functions.json");
                assert_eq!(path, "[0].registrationOrder");
            }
            other => panic!("expected JSON error, found {other:?}"),
        }
    }

    #[test]
    fn ignores_unknown_data_file_fields_for_forward_compatibility() {
        let addon = parse_json::<raw::Addon>(
            "Types.json",
            r#"{
                "name":"TestAddon",
                "version":"1.0.0",
                "futureField":{"enabled":true}
            }"#,
        )
        .unwrap();

        assert_eq!(addon.name, "TestAddon");
        assert_eq!(addon.version, "1.0.0");
    }
}

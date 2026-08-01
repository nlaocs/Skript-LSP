//! Snapshot directory I/O and the public loading pipeline.
//!
//! This module gates the schema before reading data, verifies both digests, then
//! deserializes, validates, and converts the complete snapshot atomically.

use crate::{SnapshotError, convert, digest, raw, validate};
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use syntaxes::Catalog;

/// Highest SSG snapshot schema accepted by this reader.
pub const SCHEMA_VERSION: u32 = 3;
/// Canonical manifest filename.
pub const MANIFEST_FILE: &str = "Manifest.json";

/// Data files covered by the content digest, in canonical digest order.
pub const DATA_FILES: [&str; 18] = [
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

/// Complete required schema 3 inventory, including `Manifest.json`.
pub const ALL_FILES: [&str; 19] = [
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

#[derive(Debug, Clone)]
/// Fully verified snapshot containing its source manifest and runtime catalog.
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

/// Loads and atomically validates one complete SSG schema 3 directory.
///
/// Unknown JSON fields are accepted, but required files, hashes, identities,
/// resolution states, references, and syntax patterns are verified before a
/// [`Snapshot`] is returned.
///
/// # Errors
///
/// Returns [`SnapshotError`] with file and JSON-path context for I/O, format,
/// integrity, semantic validation, or pattern failures.
pub fn load(directory: impl AsRef<Path>) -> Result<Snapshot, SnapshotError> {
    let directory = directory.as_ref();
    let manifest_text = read_file(directory, MANIFEST_FILE)?;
    let manifest: raw::Manifest = parse_json(MANIFEST_FILE, &manifest_text)?;

    if manifest.schema_version != SCHEMA_VERSION {
        return Err(SnapshotError::UnsupportedSchema {
            expected: SCHEMA_VERSION,
            actual: manifest.schema_version,
        });
    }
    validate::manifest(&manifest, &ALL_FILES)?;

    let mut serialized = BTreeMap::new();
    for file in DATA_FILES {
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
    let catalog = convert::catalog(raw_snapshot, plural_rules)?;

    Ok(Snapshot { manifest, catalog })
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

//! Typed failures produced while loading and validating an SSG snapshot.
//!
//! Errors retain the source file and JSON path whenever the failure can be tied
//! to a serialized field.

use std::path::PathBuf;

/// Failure to read, authenticate, validate, or convert an SSG snapshot.
///
/// Display messages are stable enough for diagnostics; callers should match
/// variants when behavior depends on the failure class.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)] // Variant fields are fully named and rendered by `thiserror`.
pub enum SnapshotError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid JSON in {file} at {path}: {source}")]
    Json {
        file: &'static str,
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("unsupported SSG schema version {actual}; supported range is {minimum}..={maximum}")]
    UnsupportedSchema {
        minimum: u32,
        maximum: u32,
        actual: u32,
    },
    #[error("Manifest.json files mismatch: {message}")]
    ManifestFiles { message: String },
    #[error("snapshot is missing required file {file}")]
    MissingFile { file: &'static str },
    #[error("content digest mismatch: expected {expected}, calculated {actual}")]
    ContentDigest { expected: String, actual: String },
    #[error("snapshot ID mismatch: expected {expected}, calculated {actual}")]
    SnapshotId { expected: String, actual: String },
    #[error("invalid snapshot value at {path}: {message}")]
    Validation { path: String, message: String },
    #[error("invalid syntax pattern at {path}: {source}")]
    Pattern {
        path: String,
        #[source]
        source: syntax_pattern_parser::syntax::ParseError,
    },
}

impl SnapshotError {
    pub(crate) fn validation(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Validation {
            path: path.into(),
            message: message.into(),
        }
    }
}

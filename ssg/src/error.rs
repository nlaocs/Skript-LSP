use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
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
    #[error("unsupported SSG schema version {actual}; expected {expected}")]
    UnsupportedSchema { expected: u32, actual: u32 },
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

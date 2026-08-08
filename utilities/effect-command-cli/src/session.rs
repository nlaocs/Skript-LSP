use crate::args::snapshot_directory;
use crate::report::{AnalysisReport, SnapshotDescription};
use parser_wasm::host::{HostConfig, InvocationContext, ParserHost};
use skript_parser::{
    EffectParseRequest, EffectParserConfig, ExpressionParseContext, MappedSource, RawNodeKind,
    RawTreeOptions, parse_raw_tree,
};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

const PROJECT_URI: &str = "file:///effectcommandcli";
const DOCUMENT_URI: &str = "file:///effectcommandcli/input.sk";

/// Snapshot loading, input validation, parser-host, or transaction failure.
#[derive(Debug, Error)]
pub enum EffectCommandSessionError {
    /// The schema 3 or 4 snapshot could not be loaded or validated.
    #[error("failed to load SSG snapshot {path}: {source}")]
    Snapshot {
        path: PathBuf,
        #[source]
        source: ssg::SnapshotError,
    },
    /// The manifest does not identify the Skript plugin used for lexical compatibility.
    #[error("SSG snapshot has no enabled Skript plugin entry")]
    MissingSkriptPlugin,
    /// The Skript version cannot provide the required major/minor pair.
    #[error("cannot parse Skript version {version:?} from the snapshot manifest")]
    InvalidSkriptVersion { version: String },
    /// One-shot Effect input does not contain exactly one simple Skript line.
    #[error("invalid Effect input: {message}")]
    InvalidInput { message: String },
    /// The parser worker could not be created with its bounded stack.
    #[error("failed to start Effect parser worker: {source}")]
    ParserThread {
        #[source]
        source: io::Error,
    },
    /// The parser worker panicked instead of returning a typed failure.
    #[error("Effect parser worker panicked")]
    ParserThreadPanicked,
    /// CoreLibrary or the transactional parser pipeline failed.
    #[error(transparent)]
    Host(#[from] parser_wasm::HostError),
    /// A parse transaction could not be closed after analysis.
    #[error(transparent)]
    State(#[from] parser_wasm::StateError),
}

/// Reusable SSG catalog and WASM parser host for Effect command analysis.
///
/// Loading performs full schema and digest validation once. Successive calls to
/// [`Self::analyze`] reuse the catalog and CoreLibrary host but receive distinct
/// document revisions and speculative transactions. Analysis never executes an
/// Effect and cancels every transaction after constructing the report.
pub struct EffectCommandSession {
    snapshot_path: PathBuf,
    snapshot: SnapshotDescription,
    skript_version: (u32, u32),
    catalog: Arc<syntaxes::Catalog>,
    host: ParserHost,
    next_revision: u64,
}

impl EffectCommandSession {
    /// Loads and validates one SSG schema 3 or 4 snapshot and initializes CoreLibrary.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, EffectCommandSessionError> {
        let snapshot_path = snapshot_directory(path);
        let loaded =
            ssg::load(&snapshot_path).map_err(|source| EffectCommandSessionError::Snapshot {
                path: snapshot_path.clone(),
                source,
            })?;
        let manifest = loaded.manifest();
        let skript_plugin = manifest
            .plugins
            .iter()
            .find(|plugin| plugin.enabled && plugin.name.eq_ignore_ascii_case("Skript"))
            .ok_or(EffectCommandSessionError::MissingSkriptPlugin)?;
        let skript_version = parse_skript_version(&skript_plugin.version)?;
        let snapshot = SnapshotDescription {
            snapshot_id: manifest.snapshot_id.clone(),
            minecraft_version: manifest.server.minecraft_version.clone(),
            skript_version: skript_plugin.version.clone(),
            plugin_count: manifest
                .plugins
                .iter()
                .filter(|plugin| plugin.enabled)
                .count(),
        };
        let catalog = Arc::new(loaded.into_catalog());
        let host = skript_lsp::new_parser_host(HostConfig {
            syntax_catalog: Some(Arc::clone(&catalog)),
            ..HostConfig::default()
        })?;
        Ok(Self {
            snapshot_path,
            snapshot,
            skript_version,
            catalog,
            host,
            next_revision: 1,
        })
    }

    /// Reloads the configured snapshot and rebuilds the catalog and parser host.
    pub fn reload(&mut self) -> Result<(), EffectCommandSessionError> {
        *self = Self::load(self.snapshot_path.clone())?;
        Ok(())
    }

    /// Returns the directory reloaded by the REPL's `:reload` command.
    pub fn snapshot_path(&self) -> &Path {
        &self.snapshot_path
    }

    /// Parses one complete simple line as an Effect and returns a display report.
    pub fn analyze(&mut self, input: &str) -> Result<AnalysisReport, EffectCommandSessionError> {
        std::thread::scope(|scope| {
            let worker = std::thread::Builder::new()
                .name("effectcommandcli-parser".to_owned())
                .stack_size(32 * 1024 * 1024)
                .spawn_scoped(scope, || self.analyze_inner(input))
                .map_err(|source| EffectCommandSessionError::ParserThread { source })?;
            worker
                .join()
                .map_err(|_| EffectCommandSessionError::ParserThreadPanicked)?
        })
    }

    fn analyze_inner(&mut self, input: &str) -> Result<AnalysisReport, EffectCommandSessionError> {
        if input.trim().is_empty() {
            return Err(EffectCommandSessionError::InvalidInput {
                message: "Effect text is empty".to_owned(),
            });
        }
        let source = MappedSource::identity(input);
        let tree = parse_raw_tree(
            &source,
            RawTreeOptions::for_skript_version(self.skript_version.0, self.skript_version.1),
        );
        if tree.roots.len() != 1 {
            let diagnostics = tree
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            let detail = if diagnostics.is_empty() {
                format!(
                    "expected one Effect line, found {} root nodes",
                    tree.roots.len()
                )
            } else {
                format!(
                    "expected one Effect line, found {} root nodes ({diagnostics})",
                    tree.roots.len()
                )
            };
            return Err(EffectCommandSessionError::InvalidInput { message: detail });
        }
        let node = tree
            .get(tree.roots[0])
            .expect("RawTree roots always refer to arena nodes");
        if node.kind != RawNodeKind::Simple {
            return Err(EffectCommandSessionError::InvalidInput {
                message: format!("expected a simple Effect line, found {:?}", node.kind),
            });
        }

        let revision = self.next_revision;
        self.next_revision = self.next_revision.saturating_add(1);
        let transaction = self.host.begin_parse(PROJECT_URI, DOCUMENT_URI, revision)?;
        let result = self.host.parse_effect_in_parse(
            &transaction,
            InvocationContext {
                invocation_id: revision,
                subscription_id: String::new(),
                document_id: DOCUMENT_URI.to_owned(),
                document_revision: revision,
                expansion: None,
                syntax_context: 0,
            },
            EffectParseRequest {
                source: &source,
                node,
                context: ExpressionParseContext::default(),
            },
            EffectParserConfig::default(),
        );
        let close = transaction.cancel();
        let result = result?;
        close?;
        Ok(AnalysisReport::from_result(
            input,
            &self.snapshot,
            result,
            self.catalog.as_ref(),
        ))
    }
}

fn parse_skript_version(version: &str) -> Result<(u32, u32), EffectCommandSessionError> {
    let numeric = version
        .trim()
        .strip_prefix('v')
        .unwrap_or(version.trim())
        .split(|character: char| !character.is_ascii_digit() && character != '.')
        .next()
        .unwrap_or_default();
    let mut parts = numeric.split('.');
    let major = parts.next().and_then(|part| part.parse().ok());
    let minor = parts.next().and_then(|part| part.parse().ok());
    major
        .zip(minor)
        .ok_or_else(|| EffectCommandSessionError::InvalidSkriptVersion {
            version: version.to_owned(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_release_and_prefixed_skript_versions() {
        assert_eq!(parse_skript_version("2.15.4").unwrap(), (2, 15));
        assert_eq!(parse_skript_version("v2.6.4-SNAPSHOT").unwrap(), (2, 6));
    }

    #[test]
    fn rejects_versions_without_major_and_minor() {
        assert!(matches!(
            parse_skript_version("development"),
            Err(EffectCommandSessionError::InvalidSkriptVersion { .. })
        ));
    }
}

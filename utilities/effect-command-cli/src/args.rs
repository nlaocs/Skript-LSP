use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// Human-readable command usage shared by `--help` and argument errors.
pub const HELP: &str = r#"Effect Command CLI

Parse one Skript Effect without executing it.

USAGE:
    effectcommandcli.exe [OPTIONS] [EFFECT]

ARGS:
    <EFFECT>                 Effect text to parse. Omit it to start the REPL.

OPTIONS:
    -s, --snapshot <PATH>    SSG schema 3 directory or its Manifest.json
        --json               Emit structured JSON
        --repl               Start the REPL explicitly
    -h, --help               Print help
    -V, --version            Print version

ENVIRONMENT:
    EFFECT_COMMAND_CLI_SNAPSHOT
                             Default snapshot path when --snapshot is absent

REPL COMMANDS:
    :help                    Show REPL commands
    :reload                  Reload the SSG snapshot
    :json on | :json off     Toggle JSON output
    :quit | :exit            Exit the REPL"#;

/// Output representation selected for one-shot or REPL analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Concise tree intended for terminal use.
    Human,
    /// Stable, pretty-printed JSON report.
    Json,
}

/// Execution mode selected from positional arguments and `--repl`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunMode {
    /// Parses exactly one Effect and exits.
    Once(String),
    /// Reuses one loaded snapshot and parser host for successive lines.
    Repl,
}

/// Validated runtime options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOptions {
    /// Normalized SSG snapshot directory.
    pub snapshot: PathBuf,
    /// Initial output representation.
    pub output: OutputFormat,
    /// One-shot or interactive execution.
    pub mode: RunMode,
}

/// Top-level action selected before loading a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliAction {
    /// Load a snapshot and execute the validated command.
    Run(CliOptions),
    /// Print command help.
    Help,
    /// Print the utility version.
    Version,
}

impl CliOptions {
    /// Parses command arguments without reading process globals.
    pub fn parse<I, S>(args: I, default_snapshot: PathBuf) -> Result<CliAction, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let mut snapshot = default_snapshot;
        let mut output = OutputFormat::Human;
        let mut force_repl = false;
        let mut positional = Vec::new();
        let mut values = args.into_iter().map(Into::into).peekable();
        let mut positional_only = false;

        while let Some(value) = values.next() {
            if positional_only {
                positional.push(os_to_string(value)?);
                continue;
            }
            if value == OsStr::new("--") {
                positional_only = true;
                continue;
            }
            if value == OsStr::new("-h") || value == OsStr::new("--help") {
                return Ok(CliAction::Help);
            }
            if value == OsStr::new("-V") || value == OsStr::new("--version") {
                return Ok(CliAction::Version);
            }
            if value == OsStr::new("--json") {
                output = OutputFormat::Json;
                continue;
            }
            if value == OsStr::new("--repl") {
                force_repl = true;
                continue;
            }
            if value == OsStr::new("-s") || value == OsStr::new("--snapshot") {
                snapshot = PathBuf::from(
                    values
                        .next()
                        .ok_or_else(|| "--snapshot requires a path".to_owned())?,
                );
                continue;
            }
            if let Some(encoded) = value
                .to_str()
                .and_then(|text| text.strip_prefix("--snapshot="))
            {
                if encoded.is_empty() {
                    return Err("--snapshot requires a path".to_owned());
                }
                snapshot = PathBuf::from(encoded);
                continue;
            }
            if value.to_string_lossy().starts_with('-') {
                return Err(format!("unknown option {}", value.to_string_lossy()));
            }
            positional.push(os_to_string(value)?);
        }

        if force_repl && !positional.is_empty() {
            return Err("--repl cannot be combined with an Effect argument".to_owned());
        }
        let mode = if force_repl || positional.is_empty() {
            RunMode::Repl
        } else {
            RunMode::Once(positional.join(" "))
        };
        Ok(CliAction::Run(Self {
            snapshot: snapshot_directory(snapshot),
            output,
            mode,
        }))
    }
}

fn os_to_string(value: OsString) -> Result<String, String> {
    value
        .into_string()
        .map_err(|value| format!("argument is not valid UTF-8: {}", value.to_string_lossy()))
}

pub(crate) fn snapshot_directory(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if path
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("Manifest.json"))
    {
        path.parent().unwrap_or_else(|| Path::new(".")).to_owned()
    } else {
        path.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_unquoted_effect_words_for_convenience() {
        let action =
            CliOptions::parse(["send", "1", "to", "player"], PathBuf::from("snapshot")).unwrap();
        let CliAction::Run(options) = action else {
            panic!("command must run");
        };
        assert_eq!(options.mode, RunMode::Once("send 1 to player".to_owned()));
    }

    #[test]
    fn no_effect_selects_repl_and_manifest_selects_parent() {
        let action = CliOptions::parse(
            ["--snapshot", "generated/Manifest.json", "--json"],
            PathBuf::from("unused"),
        )
        .unwrap();
        let CliAction::Run(options) = action else {
            panic!("command must run");
        };
        assert_eq!(options.mode, RunMode::Repl);
        assert_eq!(options.output, OutputFormat::Json);
        assert_eq!(options.snapshot, PathBuf::from("generated"));
    }

    #[test]
    fn explicit_repl_rejects_effect_text() {
        let error = CliOptions::parse(["--repl", "send 1"], PathBuf::from("snapshot")).unwrap_err();
        assert!(error.contains("cannot be combined"));
    }
}

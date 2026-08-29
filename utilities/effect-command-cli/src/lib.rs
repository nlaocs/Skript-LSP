#![doc = include_str!("../README.md")]
#![warn(rustdoc::broken_intra_doc_links)]

mod args;
mod repl;
mod report;
mod session;

pub use args::{CliAction, CliOptions, OutputFormat, RunMode};
pub use report::AnalysisReport;
pub use session::{EffectCommandSession, EffectCommandSessionError};

use std::ffi::OsString;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;

/// Stable exit code for a successfully matched Effect or a clean REPL exit.
pub const EXIT_SUCCESS: u8 = 0;
/// Stable exit code for valid input that matches no registered Effect.
pub const EXIT_NO_MATCH: u8 = 1;
/// Stable exit code for invalid CLI arguments.
pub const EXIT_USAGE: u8 = 2;
/// Stable exit code for snapshot, parser-host, or I/O setup failures.
pub const EXIT_FAILURE: u8 = 3;

/// Runs the CLI with process arguments, standard streams, and the configured snapshot default.
///
/// `EFFECT_COMMAND_CLI_SNAPSHOT` takes precedence over the current directory.
/// The function returns a stable process code instead of terminating, which
/// keeps the binary entry point small and lets tests exercise the same flow.
pub fn run_from_environment() -> u8 {
    let default_snapshot = std::env::var_os("EFFECT_COMMAND_CLI_SNAPSHOT")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let stdin = io::stdin();
    let stdout = io::stdout();
    let stderr = io::stderr();
    let color = stdout.is_terminal() && std::env::var_os("NO_COLOR").is_none();
    run_with_io_mode(
        std::env::args_os().skip(1),
        default_snapshot,
        stdin.lock(),
        stdout.lock(),
        stderr.lock(),
        color,
    )
}

/// Runs the complete command with caller-owned arguments and streams.
///
/// This is the testable integration boundary for both one-shot and REPL mode.
/// `default_snapshot` is used when `--snapshot` is absent.
pub fn run_with_io<I, S, R, W, E>(
    args: I,
    default_snapshot: PathBuf,
    input: R,
    output: W,
    error: E,
) -> u8
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
    R: BufRead,
    W: Write,
    E: Write,
{
    run_with_io_mode(args, default_snapshot, input, output, error, false)
}

fn run_with_io_mode<I, S, R, W, E>(
    args: I,
    default_snapshot: PathBuf,
    input: R,
    mut output: W,
    mut error: E,
    color: bool,
) -> u8
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
    R: BufRead,
    W: Write,
    E: Write,
{
    let action = match CliOptions::parse(args, default_snapshot) {
        Ok(action) => action,
        Err(message) => {
            let _ = writeln!(error, "error: {message}");
            let _ = writeln!(error, "\n{}", args::HELP);
            return EXIT_USAGE;
        }
    };

    let options = match action {
        CliAction::Help => {
            let _ = writeln!(output, "{}", args::HELP);
            return EXIT_SUCCESS;
        }
        CliAction::Version => {
            let _ = writeln!(output, "effectcommandcli {}", env!("CARGO_PKG_VERSION"));
            return EXIT_SUCCESS;
        }
        CliAction::Run(options) => options,
    };

    let mut session = match EffectCommandSession::load(&options.snapshot) {
        Ok(session) => session,
        Err(load_error) => {
            let _ = writeln!(error, "error: {load_error}");
            return EXIT_FAILURE;
        }
    };

    match options.mode {
        RunMode::Once(effect) => match session.analyze(&effect) {
            Ok(report) => {
                let matched = report.matched();
                if let Err(render_error) =
                    report.write_with_color(options.output, &mut output, color)
                {
                    let _ = writeln!(error, "error: failed to write output: {render_error}");
                    return EXIT_FAILURE;
                }
                if matched { EXIT_SUCCESS } else { EXIT_NO_MATCH }
            }
            Err(parse_error) => {
                let _ = writeln!(error, "error: {parse_error}");
                EXIT_FAILURE
            }
        },
        RunMode::Repl => repl::run(&mut session, options.output, input, output, error, color),
    }
}

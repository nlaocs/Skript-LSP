use crate::{EXIT_FAILURE, EXIT_SUCCESS, EffectCommandSession, OutputFormat};
use std::io::{self, BufRead, Write};

const REPL_HELP: &str = r#"REPL commands:
  :help                 Show this command list
  :reload               Reload the configured SSG snapshot
  :json on              Use JSON reports
  :json off             Use human-readable reports
  :quit, :exit          Exit the REPL

Every other non-empty line is parsed as one Effect and is never executed."#;

pub(crate) fn run<R: BufRead, W: Write, E: Write>(
    session: &mut EffectCommandSession,
    mut format: OutputFormat,
    mut input: R,
    mut output: W,
    mut error: E,
    color: bool,
) -> u8 {
    if writeln!(output, "Effect Command CLI")
        .and_then(|_| writeln!(output, "Type :help for available commands."))
        .is_err()
    {
        return EXIT_FAILURE;
    }
    let mut line = String::new();
    loop {
        if write!(output, "\neffect> ")
            .and_then(|_| output.flush())
            .is_err()
        {
            return EXIT_FAILURE;
        }
        line.clear();
        match input.read_line(&mut line) {
            Ok(0) => {
                let _ = writeln!(output);
                return EXIT_SUCCESS;
            }
            Ok(_) => {}
            Err(read_error) if read_error.kind() == io::ErrorKind::Interrupted => {
                let _ = writeln!(output, "^C");
                continue;
            }
            Err(read_error) => {
                let _ = writeln!(error, "error: failed to read REPL input: {read_error}");
                return EXIT_FAILURE;
            }
        }
        let effect = line.trim_end_matches(['\r', '\n']);
        if effect.trim().is_empty() {
            continue;
        }
        if effect.starts_with(':') {
            match repl_command(effect, session, &mut format, &mut output) {
                ReplControl::Continue => continue,
                ReplControl::Exit => return EXIT_SUCCESS,
            }
        }
        match session.analyze(effect) {
            Ok(report) => {
                if let Err(write_error) = report.write_with_color(format, &mut output, color) {
                    let _ = writeln!(error, "error: failed to write output: {write_error}");
                    return EXIT_FAILURE;
                }
            }
            Err(parse_error) => {
                let _ = writeln!(output, "error: {parse_error}");
            }
        }
    }
}

enum ReplControl {
    Continue,
    Exit,
}

fn repl_command(
    command: &str,
    session: &mut EffectCommandSession,
    format: &mut OutputFormat,
    output: &mut dyn Write,
) -> ReplControl {
    match command.trim() {
        ":quit" | ":exit" => ReplControl::Exit,
        ":help" => {
            let _ = writeln!(output, "{REPL_HELP}");
            ReplControl::Continue
        }
        ":reload" => {
            match session.reload() {
                Ok(()) => {
                    let _ = writeln!(output, "reloaded {}", session.snapshot_path().display());
                }
                Err(error) => {
                    let _ = writeln!(output, "error: {error}");
                }
            }
            ReplControl::Continue
        }
        ":json on" => {
            *format = OutputFormat::Json;
            let _ = writeln!(output, "JSON output enabled");
            ReplControl::Continue
        }
        ":json off" => {
            *format = OutputFormat::Human;
            let _ = writeln!(output, "JSON output disabled");
            ReplControl::Continue
        }
        ":json" => {
            let enabled = matches!(format, OutputFormat::Json);
            let _ = writeln!(
                output,
                "JSON output is {}",
                if enabled { "enabled" } else { "disabled" }
            );
            ReplControl::Continue
        }
        unknown => {
            let _ = writeln!(
                output,
                "unknown REPL command {unknown:?}; type :help for available commands"
            );
            ReplControl::Continue
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_documents_every_required_command() {
        for command in [":help", ":reload", ":json on", ":json off", ":quit"] {
            assert!(REPL_HELP.contains(command));
        }
    }
}

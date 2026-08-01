use effect_command_cli::run_from_environment;
use std::process::ExitCode;

fn main() -> ExitCode {
    ExitCode::from(run_from_environment())
}

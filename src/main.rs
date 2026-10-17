use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    match arguments.as_slice() {
        [] => {}
        [argument] if argument == "--stdio" => {}
        [argument] if argument == "--help" || argument == "-h" => {
            println!(
                "skript-lsp [--stdio]\n\nSet initializationOptions.snapshotPath and a single project rootUri in your LSP client."
            );
            return ExitCode::SUCCESS;
        }
        _ => {
            eprintln!("usage: skript-lsp [--stdio]");
            return ExitCode::FAILURE;
        }
    }
    match skript_lsp::lsp::run_stdio() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("skript-lsp: {error}");
            ExitCode::FAILURE
        }
    }
}

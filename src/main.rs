//! Minimal executable scaffold for the future language server.
//!
//! The binary currently forces the embedded CoreLibrary artifact to be linked
//! and provides a smoke-test entry point. LSP transport and document lifecycle
//! integration belong here once those layers are implemented.

fn main() {
    std::hint::black_box(skript_lsp::core_library_component());
    println!("Hello, world!");
}

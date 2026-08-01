fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        // Full multi-addon rejection explores deeply nested registered
        // Expressions through Wasmtime. Reserve enough stack for the Windows
        // executable without changing parser limits or downstream libraries.
        println!("cargo:rustc-link-arg-bin=effectcommandcli=/STACK:67108864");
    }
}

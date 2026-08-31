#![doc = include_str!("../README.md")]
#![warn(rustdoc::broken_intra_doc_links)]

/// The mandatory CoreLibrary component bundled into the LSP executable.
///
/// A missing artifact is a compile-time error. Rebuild it with
/// `cargo run -p xtask -- build-core-library`.
pub static CORE_LIBRARY_COMPONENT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/artifacts/core-library.wasm"
));

/// Returns the bytes of the mandatory CoreLibrary WebAssembly Component.
///
/// The returned slice is embedded into the executable at compile time and is
/// therefore valid for the lifetime of the process. The artifact must be built
/// with cargo run -p xtask -- build-core-library before this crate is compiled.
///
/// # Examples
///
/// ~~~
/// let component = skript_lsp::core_library_component();
/// assert!(!component.is_empty());
/// assert_eq!(&component[..4], b"\0asm");
/// ~~~
pub fn core_library_component() -> &'static [u8] {
    CORE_LIBRARY_COMPONENT
}

/// Creates the parser host with the mandatory bundled CoreLibrary loaded.
///
/// This is the executable crate's normal entry point into parser-wasm. It
/// guarantees that [parser_wasm::ParserHost::components] starts with
/// nlaocs.core-library and avoids making callers locate the generated artifact.
///
/// # Examples
///
/// ~~~no_run
/// let config = parser_wasm::HostConfig {
///     runtime_profile: parser_wasm::RuntimeProfile {
///         skript_version: Some("2.15.4".to_owned()),
///         ..parser_wasm::RuntimeProfile::default()
///     },
///     ..parser_wasm::HostConfig::default()
/// };
/// let host = skript_lsp::new_parser_host(config)?;
///
/// assert_eq!(host.components()[0].component_id, "nlaocs.core-library");
/// # Ok::<(), parser_wasm::HostError>(())
/// ~~~
///
/// # Errors
///
/// Returns [parser_wasm::HostError] when host limits are invalid or the bundled
/// component cannot be compiled, negotiated, initialized, or registered.
pub fn new_parser_host(
    config: parser_wasm::HostConfig,
) -> Result<parser_wasm::ParserHost, parser_wasm::HostError> {
    parser_wasm::ParserHost::new(CORE_LIBRARY_COMPONENT, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wit_component::DecodedWasm;

    #[test]
    fn bundled_core_library_is_a_parser_addon_component() {
        let decoded = wit_component::decode(CORE_LIBRARY_COMPONENT).expect("artifact must decode");
        let DecodedWasm::Component(resolve, world_id) = decoded else {
            panic!("bundled CoreLibrary must be a Component");
        };
        let world = &resolve.worlds[world_id];
        assert_eq!(world.exports.len(), 5);
    }

    #[test]
    fn bundled_core_library_initializes_the_parser_host() {
        let host = new_parser_host(parser_wasm::HostConfig {
            runtime_profile: parser_wasm::RuntimeProfile {
                skript_version: Some("2.15.4".to_owned()),
                ..parser_wasm::RuntimeProfile::default()
            },
            ..parser_wasm::HostConfig::default()
        })
        .expect("bundled CoreLibrary must initialize");
        let components = host.components();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].component_id, "nlaocs.core-library");
    }
}

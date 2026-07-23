/// The mandatory CoreLibrary component bundled into the LSP executable.
///
/// A missing artifact is a compile-time error. Rebuild it with
/// `cargo run -p xtask -- build-core-library`.
pub static CORE_LIBRARY_COMPONENT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/artifacts/core-library.wasm"
));

pub fn core_library_component() -> &'static [u8] {
    CORE_LIBRARY_COMPONENT
}

/// Creates the parser host with the mandatory bundled CoreLibrary loaded.
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
        let host = new_parser_host(parser_wasm::HostConfig::default())
            .expect("bundled CoreLibrary must initialize");
        let components = host.components();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].component_id, "nlaocs.core-library");
    }
}

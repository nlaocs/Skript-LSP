//! Generated Wasmtime Component Model bindings for the parser-addon WIT world.
//!
//! Do not add hand-authored domain behavior here. Stable native entry points live in
//! the crate root, `host`, and `state` modules.

wasmtime::component::bindgen!({
    path: "wit",
    world: "parser-addon",
});

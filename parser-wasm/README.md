# Parser WASM ABI

This crate owns the Component Model contract between the Rust parser host,
the required CoreLibrary component, and parser Addons.

The WIT package is `nlaocs:skript-parser-addon@0.1.0`. Its
`parser-addon` world exports five typed interfaces:

- `addon`: static manifest and host-profile negotiation
- `hooks`: parser-stage observation, transformation, and override
- `text-macro`: edits over virtual UTF-8 source text
- `tree-macro`: replacement of the indentation-based RawTree
- `ast-macro`: replacement of the parsed AST arena

All parser payloads are WIT records and variants. JSON is not part of the ABI.
RawTree and AST values use node-ID arenas so their payloads remain non-recursive
Component Model values.

## Compatibility

Every manifest exposes a component ID and component version for diagnostics.

The package version identifies the WIT shape. The manifest's `abi` field is a
runtime handshake and currently requires an exact `major.minor` match.

Capabilities use stable string IDs and independent integer versions instead of
a closed enum. This allows a newer component to describe a capability to an
older host without failing while lifting its manifest.

- A missing or older required capability rejects initialization.
- A missing or older optional capability is ignored.
- Duplicate or blank capability IDs are invalid.
- Both the host and guest use the same negotiation rule. The host validates the
  component manifest, then the guest validates the host profile in
  `addon.initialize`.

## Hook rules

A subscription selects a target, phase, signed priority, and mode.

- `observe` reads a payload but must not replace it.
- `transform` may return a replacement payload for later hooks.
- `override` handles the target instead of its normal implementation.

The host validates mode-specific behavior, payload variants, subscriptions, and
capabilities when components are registered. Runtime limits and trap handling
belong to the Wasmtime host implementation.

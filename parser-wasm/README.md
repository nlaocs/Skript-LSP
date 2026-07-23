# Parser WASM ABI

[日本語](README.ja.md)

`parser-wasm` owns both sides of the WebAssembly Component Model boundary
between the Rust parser, mandatory CoreLibrary, and parser addon components.
It contains the shared ABI model plus the optional native Wasmtime host.

It does not parse Skript patterns or `.sk` source. It transports typed parser
data, orders hook calls, enforces limits, and makes accepted component side
effects transactional.

## Feature Modes

The default `host` feature enables Wasmtime, persistence, URL handling, and the
`syntaxes` integration used by the LSP:

```toml
parser-wasm = { path = "../parser-wasm" }
```

WASM guest crates should disable default features:

```toml
parser-wasm = { path = "../parser-wasm", default-features = false }
```

That mode exposes ABI versions, capability IDs, and compatibility validation
without linking the native host.

## WIT Contract

The WIT package is `nlaocs:skript-parser-addon@0.1.0`. Its
`parser-addon` world imports host services and exports guest implementations.

Guest exports:

- `addon`: static manifest and host-profile negotiation
- `hooks`: parser-stage observation, transformation, and override
- `text-macro`: edits over virtual UTF-8 source text
- `tree-macro`: replacement of the indentation-based RawTree
- `ast-macro`: replacement of the parsed AST arena

Host imports:

- `state-store`: scoped key/value storage with compare-and-swap and prefix scan
- `dynamic-syntax-registry`: add, override, and remove syntax definitions

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

The WIT contract defines text, tree, and AST macro capabilities, but the current
host does not advertise or execute those macro pipelines yet. Constants exist
so the ABI can be implemented incrementally without changing capability IDs.

## Hook rules

A subscription selects a target, phase, signed priority, and mode.

- `observe` reads a payload but must not replace it.
- `transform` may return a replacement payload for later hooks.
- `override` handles the target instead of its normal implementation.

The host validates mode-specific behavior, payload variants, subscriptions, and
capabilities when components are registered. Runtime limits and trap handling
belong to the Wasmtime host implementation.

Subscription ordering is deterministic:

1. exact registration targets before syntax-kind targets before parse-stage
   targets
2. signed subscription priority
3. component load order
4. declaration order inside the component manifest

A handled override stops later matching hooks. Addon errors are reported as
component failures. Traps, timeouts, fuel exhaustion, and resource-limit
violations disable the component.

## StateStore

StateStore is a host import available to hooks and future macro calls.

| Scope | Lifetime |
| --- | --- |
| `invocation` | One component call |
| `parse` | One parse transaction |
| `document` | Committed document revisions |
| `project` | Documents and addons in one project |
| `persistent-project` | Project state stored across LSP restarts |

Namespaces are private to one component or shared through an explicit schema
declaration. Shared declarations specify schema ID, schema version, readers,
and writers. Values use a declared raw, CBOR, or JSON encoding, but the host
does not interpret their bytes.

Each component call receives an invocation overlay. Rejected, trapped, or
invalid calls roll it back. Accepted calls merge into the parse overlay. A
parse commits only when its document revision is still current and no project
revision conflict occurred. Persistent project state uses `redb` below the OS
application data directory, partitioned by canonical project URI.

## Dynamic Syntax

When `HostConfig::syntax_catalog` contains an SSG-backed
`Arc<syntaxes::Catalog>`, the host advertises `parser.dynamic-syntax`.
Components may then:

- register new syntax with namespaced component/local IDs
- override static syntax by definition ID or registration ID
- add ordering constraints against static or dynamic syntax
- remove their own dynamic entries

Updates are allowed during component initialization and Document/Preprocess
prepass. The registry freezes before later parser phases. Its immutable
snapshot combines static and dynamic candidates in deterministic topological
order. Rejection and host failure roll dynamic updates back alongside
StateStore updates. Unloading a component removes its entries from future
snapshots without mutating already frozen document snapshots.

The capability is intentionally unavailable when no Catalog is connected.

## Native Host API

The main entry points are:

- `ParserHost::new`: instantiate the mandatory CoreLibrary
- `load_addon` and `unload_addon`: manage component lifecycles
- `begin_parse`: create a multi-phase parse transaction
- `dispatch_in_parse`: invoke matching hook subscriptions
- `dynamic_syntax_snapshot`: freeze and retrieve ranked syntax candidates
- `dispatch`: convenience API for a one-dispatch transaction

`HostConfig` controls call fuel, epoch timeout, Wasmtime memory/table/instance
limits, dispatch output quotas, StateStore configuration, and the optional
syntax Catalog.

## Source Layout

| Path | Responsibility |
| --- | --- |
| `wit/` | Component Model package, world, records, variants, and host imports |
| `src/bindings.rs` | Wasmtime bindings generated from WIT |
| `src/host.rs` | component lifecycle, subscriptions, dispatch, limits, and dynamic syntax bridge |
| `src/state/mod.rs` | namespace registry and in-memory transactional StateStore |
| `src/state/persistent.rs` | `redb` persistent-project backend |
| `tests/contract.rs` | host and guest binding contract |
| `tests/host.rs` | CoreLibrary lifecycle and Wasmtime behavior |
| `tests/state.rs` | scopes, permissions, conflicts, quotas, and persistence |
| `tests/dynamic_syntax.rs` | real WASM dynamic registration against an SSG fixture |

## Testing

Build embedded components before integration tests:

```sh
cargo run -p xtask --locked -- build-core-library
cargo run -p xtask --locked -- build-test-components
cargo test -p parser-wasm --locked
```

The complete workspace check also verifies that host-only dependencies are not
accidentally required by guest components.

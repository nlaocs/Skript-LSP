# Effect Command CLI

[日本語](README.ja.md)

`effectcommandcli` is a standalone inspection utility that parses one Skript
Effect against an exact SkriptSyntaxGenerator (SSG) schema 3 or 4 snapshot. It never
executes the Effect. The binary demonstrates how `ssg`, `syntaxes`,
`skript-parser`, `parser-wasm`, and the mandatory CoreLibrary fit together.

## Build

CoreLibrary is embedded in the executable, so build its Component artifact
first:

```console
cargo run -p xtask --locked -- build-core-library
cargo build -p effect-command-cli --locked
```

The Windows executable is `target/debug/effectcommandcli.exe`.

## Snapshot

Pass either an SSG output directory or its `Manifest.json`:

```console
effectcommandcli.exe --snapshot C:\server\plugins\SkriptSyntaxGenerator "send 1"
```

When `--snapshot` is omitted, the utility uses
`EFFECT_COMMAND_CLI_SNAPSHOT`, then the current directory. The complete
snapshot is validated before CoreLibrary starts; unsupported schemas, digest
mismatches, missing files, and invalid cross-file references fail before
parsing.

## One-Shot Mode

An Effect argument parses one line and exits:

```console
effectcommandcli.exe "send 1"
effectcommandcli.exe --json "broadcast \"hello\""
effectcommandcli.exe "send sin(abs(-1))"
```

Human output identifies the selected Effect, addon, implementation class,
registration pattern, pattern AST, captures, expected Skript types, resolved
Java return types, multiplicity, nested Expressions, parse tags, parse marks,
alternatives, and the farthest useful failure. JSON reports carry
`schemaVersion: 3` so consumers can version their reader independently from the
SSG schema. Human reports include `parseTime` in milliseconds for durations of
at least one millisecond and in nanoseconds for shorter parses. JSON reports
expose the duration as integer nanoseconds in `parseDurationNs`. The duration
covers parsing only; loading and indexing the SSG snapshot is excluded.

`patternElements` is the complete AST of the selected registration pattern,
including branches that were not selected. `elements` contains the regex and typed
Expression captures that actually participated in the match.

Some addons intentionally register catch-all Effects. For example,
skript-reflect registers an expression statement as `[1:await] <.+>`, so any
non-empty input may be a valid Effect in snapshots containing that addon. The CLI
reports the selected catch-all instead of manufacturing an unknown result.

When a registration matches a meaningful prefix but one typed capture fails,
the result is `incomplete`. It preserves the Effect identity and underlines the
failed capture; `unknown` is reserved for input with no recognizable candidate.

Exit codes are stable:

| Code | Meaning |
| ---: | --- |
| `0` | A registered Effect matched. |
| `1` | The input was valid but no Effect matched. |
| `2` | CLI arguments were invalid. |
| `3` | Snapshot, host, parser, or stream setup failed. |

## REPL Mode

Omit the Effect or pass `--repl` to reuse the loaded snapshot, catalog, and
parser host:

```console
effectcommandcli.exe --snapshot C:\server\plugins\SkriptSyntaxGenerator

effect> send 1
effect> broadcast "hello"
effect> :json on
effect> :reload
effect> :quit
```

Available commands are `:help`, `:reload`, `:json on`, `:json off`, `:quit`,
and `:exit`. A no-match or malformed line is reported without ending the REPL.
EOF exits cleanly; an interrupted read returns to the prompt.

## Current Boundary

SSG-registered Skript and addon Functions are parsed as structured Expression
nodes. Reports include the Function name, definition/registration IDs, addon,
return type, multiplicity, declared parameter names, named bindings, omitted
optional parameters, and recursively parsed argument Expressions. Opaque WASM
Function leaves remain distinguishable with `structured: false`.

User Functions declared inside `.sk` files are not registered yet. The parser
already accepts document definitions through `lookup_functions`; declaration
collection and project symbol management will connect that source after whole
file Structure parsing. The remaining CLI work stays tracked by
[Issue #79](https://github.com/nlaocs/Skript-LSP/issues/79).

The utility parses exactly one top-level Effect line. It does not parse a whole
`.sk` file, run Text/Tree macros, or execute Minecraft behavior.

## Tests

```console
cargo test -p effect-command-cli --locked
```

Integration tests use both the checked-in multi-addon Skript 2.15.4 and legacy
Skript 2.6.4/Minecraft 1.12.2 schema 3 snapshots. They cover one-shot JSON,
unknown Effects, nested Function/Expression data, REPL continuation, output
switching, and snapshot reload.

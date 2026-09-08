# Effect Command CLI

[日本語](README.ja.md)

Schema 7 displays implicit/default Expression children and rejected/unresolved
omitted arguments in both human and JSON reports. For example, `send stone`
requires a CommandSender-providing Event such as `--event "on join"`;
`send stone to console` has an explicit recipient. Source text is preserved.
See the [shared DefaultExpression model](../../docs/default-expressions.md).

`effectcommandcli` is a standalone inspection utility that parses one Skript
Effect against an exact SkriptSyntaxGenerator (SSG) schema 3, 4, or 5 snapshot. It never
executes the Effect. The binary demonstrates how `ssg`, `syntaxes`,
`skript-parser`, `parser-wasm`, and the mandatory CoreLibrary fit together.

## Build

CoreLibrary is embedded in the executable, so build its Component artifact
first:

```console
rustup target add wasm32-unknown-unknown
cargo run -p xtask --locked -- build-core-library
cargo build -p effect-command-cli --locked
```

The Windows executable is `target/debug/effectcommandcli.exe`.

## Snapshot

Pass either an SSG output directory or its `Manifest.json`:

```console
effectcommandcli.exe --snapshot C:\server\plugins\SkriptSyntaxGenerator "send 1 to console"
```

When `--snapshot` is omitted, the utility uses
`EFFECT_COMMAND_CLI_SNAPSHOT`, then the current directory. The complete
snapshot is validated before CoreLibrary starts; unsupported schemas, digest
mismatches, missing files, and invalid cross-file references fail before
parsing.

Schema 5 requires `Language.json`; schemas 3 and 4 do not. See the
[`ssg` format documentation](../../ssg/README.md) for the required file inventories.

## One-Shot Mode

An Effect argument parses one line and exits:

```console
effectcommandcli.exe "send 1 to console"
effectcommandcli.exe --json "broadcast \"hello\""
effectcommandcli.exe "send sin(abs(-1)) to console"
effectcommandcli.exe --event "on join:" "send join message"
effectcommandcli.exe --section "loop all players:" "continue"
effectcommandcli.exe --section "loop all players" --section "if loop-player is online:" "exit 2 sections"
```

`--event <HEADER>` parses the Effect inside a selected Skript Event. The
header is matched through StructEvent and the snapshot's Event catalog; the
trailing `:` is optional, and `on join`, `on join:`, and `join` select the same
Event. The resulting Event classes and Event values are included in both
human and JSON reports. Human output shows the EventValue count; JSON retains
each EventValue's SSG registration, ordering, changer, validator, exclusion,
pattern, and addon metadata.

Repeat `--section <HEADER>` from the outermost Section to the innermost to
analyze an Effect inside an artificial Section stack. Each header is parsed by
the normal Section parser; it is not stored as a free-form label. A trailing
`:` is optional. The selected registration identity, addon, flags, captures,
return types, multiplicity, and addon metadata are available to CoreLibrary and
other WASM hooks through a parser-owned read-only scope stack. JSON reports
expose the same data under `context.sections`.
Selecting a Section retains its enter-hook transaction state instead of running
the root exit hook immediately. `pop` and `clear` restore the saved transaction,
so stateful WASM addons observe the same scope lifetime as later Effect parses.
Dynamic registrations report their `ownerComponentId` separately from catalog
addon metadata.

Human output identifies the selected Effect, addon, implementation class,
registration pattern, pattern AST, captures, expected Skript types, resolved
Java return types, multiplicity, nested Expressions, public semantic data,
parse tags, parse marks, alternatives, and the farthest useful failure. JSON
reports carry `schemaVersion: 7` so consumers can version their reader
independently from the SSG schema. Human reports include `parseTime` in
milliseconds for durations of
at least one millisecond and in nanoseconds for shorter parses. JSON reports
expose the duration as integer nanoseconds in `parseDurationNs`. The duration
covers RawTree parsing, parser analysis, and transaction rollback. Snapshot
loading, indexing, report construction, and rendering are excluded.

Each resolved Expression reports its node-local `publicData` records beside
`metadata`. A record has `schemaId`, `schemaVersion`, and `json`; valid JSON is
emitted as a structured raw value rather than a JSON string, so large integer
and decimal spellings are preserved. Nested children keep their own records,
including the empty list on a grouped wrapper. Human output shows the same
schema/version and JSON object without changing the rendered source.

Human parse failures use `miette` to label the farthest failure span directly
in the source. Human formatting may evolve for readability; JSON output is the
stable machine-readable contract and changes only with `schemaVersion`.

`patternElements` is the AST of the selected registration pattern, including
branches that were not selected. Report rendering is bounded: pattern AST
recursion is truncated at depth 16, while nested Expression data is truncated at
depth 8. `elements` contains the regex and typed Expression captures that
actually participated in the match.

Static SSG EffectSection registrations are candidates before ordinary Effects
and are reported with their Section syntax
identity; ordinary Section registrations are not treated as Effects. The JSON
report does not add a separate `effectSection` field.

Human output is colored only when stdout is a terminal and `NO_COLOR` is absent.

Some addons intentionally register catch-all Effects. For example,
skript-reflect registers an expression statement as `[1:await] <.+>`. Its presence
in the snapshot alone is insufficient: a loaded WASM component must support the
regex capture and validate the input. Native matching excludes regex syntax
without a WASM route; a broad pattern does not make every non-empty input valid.

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

effect> send 1 to console
effect> broadcast "hello"
effect> :event on join:
effect> :section loop all players:
effect> send join message
effect> :context
effect> :section pop
effect> :section clear
effect> :event off
effect> :json on
effect> :reload
effect> :quit
```

Available commands are `:help`, `:reload`, `:event <HEADER>`, `:event off`,
`:events`, `:section <HEADER>`, `:section pop`, `:section clear` (or `off`),
`:context`, `:json on`, `:json off`, `:quit`, and `:exit`. `:events`
lists both SSG catalog Events and Events registered dynamically by WASM addons.
Event selection always uses a real Skript Event header so StructEvent and addon
WASM hooks observe the same input. Section commands push, pop, or clear the
parser-owned stack without changing the selected Event. Selecting another
Event clears the Section stack because it starts a new root context. A no-match
or malformed line is reported without ending the REPL.
EOF exits cleanly; an interrupted read returns to the prompt.

## Current Boundary

SSG-registered Skript and addon Functions are parsed as structured Expression
nodes. Reports include the Function name, definition/registration IDs, addon,
return type, multiplicity, declared parameter names, named bindings, omitted
optional parameters, and recursively parsed argument Expressions. Opaque WASM
Function leaves remain distinguishable with `structured: false`.

The libraries already collect document Function declarations through two-pass
Structure parsing and expose them through `lookup_functions`. This one-line CLI
does not load those declarations, so user-defined Functions are unavailable in
its session. Project-wide symbol management is also not implemented. Remaining
CLI work stays tracked by
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
switching, Event-context selection, nested Section-context push/pop/clear,
loop-scoped Effects and Expressions, and snapshot reload.

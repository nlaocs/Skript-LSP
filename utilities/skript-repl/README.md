# Skript REPL

[Japanese](README.ja.md)

`skript-repl` is a standalone parser console backed by an exact
SkriptSyntaxGenerator (SSG) snapshot and the same WASM parser host used by
Skript-LSP. It can inspect either one Effect or a submitted multiline Skript
document. It never executes Skript or connects to a Minecraft server.

The REPL keeps the loaded catalog, CoreLibrary, optional parser addons, and
committed StateStore data between inputs. This makes it a small interactive
frontend for the shared parser rather than an Effect-specific parser.

## Build

CoreLibrary is embedded in the executable, so build its Component first:

```console
rustup target add wasm32-unknown-unknown
cargo run -p xtask --locked -- build-core-library
cargo build -p skript-repl --locked
```

The Windows executable is `target/debug/skript-repl.exe`.

## Snapshot And Addons

Pass either an SSG output directory or its `Manifest.json`:

```console
skript-repl.exe --snapshot C:\server\plugins\SkriptSyntaxGenerator --repl
```

When `--snapshot` is omitted, `SKRIPT_REPL_SNAPSHOT` is used, followed by the
current directory. The snapshot is fully validated before CoreLibrary starts.
SSG schemas 3 through 7 are supported. Schemas 5 and later require
`Language.json`; schema 7 also requires `BlockData.json`.

Additional parser addon Components can be loaded in command-line order:

```console
skript-repl.exe --snapshot C:\snapshot \
  --addon C:\addons\reflect-parser.wasm \
  --addon C:\addons\project-rules.wasm \
  --repl
```

`--addon` is repeatable. An addon receives the same runtime profile, SSG
catalog access, hooks, dynamic syntax registry, and transactional StateStore as
CoreLibrary.

## One-Shot Effect

Providing an Effect parses one simple line and exits:

```console
skript-repl.exe "send 1 to console"
skript-repl.exe --json "broadcast \"hello\""
skript-repl.exe --event "on join:" "send player"
skript-repl.exe --section "loop all players:" "continue"
```

This mode preserves the former EffectCommandCLI contract. It reports the
selected Effect or EffectSection, pattern, captures, recursive Expressions,
types, multiplicity, addon data, defaults, diagnostics, and parse time. Its
parse transaction is discarded after reporting, including successful hook
writes.

Stable exit codes are:

| Code | Meaning |
| ---: | --- |
| `0` | The Effect matched. |
| `1` | The input was valid but no Effect matched. |
| `2` | Command-line arguments were invalid. |
| `3` | Snapshot, host, parser, addon, or I/O setup failed. |

## Interactive REPL

Omit the Effect or pass `--repl`:

```console
skript-repl.exe --snapshot C:\snapshot --repl
```

A simple line is still parsed immediately:

```console
skript> send 1 to console
```

A Structure or Section header starts a document draft. The terminal proposes
the next indentation, but the user can edit it freely:

```console
skript> on join:
......>     send "hello" to console
......>     loop all players:
......>         send loop-player's name to console
......>     send "done" to console
......>
```

Submit a draft with an empty line or `:submit`. `:cancel` and Ctrl+C discard
only the current draft. EOF submits a non-empty draft, then exits. Multiline
terminal paste is retained as one draft, including internal blank lines and
block comments. The original whitespace and line endings supplied to the
parser are not normalized to make invalid input pass.

Submitted documents use `ParserHost::parse_document`, including Text and Tree
macros, RawTree construction, two-pass Structure parsing, Event/Section scopes,
recursive Effect/Condition/Expression parsing, AST macros, diagnostics, and
transactional commit. Recoverable failures produce a partial tree and leave the
REPL running.

Successful document inputs commit parser-addon StateStore writes. Successful
one-line REPL Effects also retain their selected branch's writes; rejected or
incomplete candidates are rolled back. This permits a future addon to model
parse-time declarations such as an `import ...` Effect and expose the imported
symbols to later inputs without adding addon-specific behavior to the REPL.
Cancelled and never-submitted drafts never enter the parser and cannot change
state.

## REPL Commands

| Command | Action |
| --- | --- |
| `:help` | Show the command list. |
| `:reload` | Reload the snapshot and every `--addon` Component. |
| `:event <header>` | Select an Event context. A trailing `:` is optional. |
| `:event off` | Clear the Event and Section contexts. |
| `:events` | List catalog and dynamically registered Events. |
| `:section <header>` | Push a parsed Section context. A trailing `:` is optional. |
| `:section pop` | Pop the innermost Section context. |
| `:section clear` | Clear all Section contexts. |
| `:context` | Show the active manual contexts. |
| `:json on`, `:json off` | Change report format. |
| `:submit` | Submit the current document draft. |
| `:cancel` | Discard the current document draft. |
| `:quit`, `:exit` | Exit. |

Commands are not inserted into an active document. REPL meta commands remain
active even while an unclosed `###` block comment is being collected, so
`:submit` can diagnose it and `:cancel` can escape it. Paste a multiline block
when the Skript source itself must contain an exact line such as `:submit`.
`:event` and `:section`
apply only to one-line Effects in an artificial body context. Multiline
documents do not inherit those manual contexts and are parsed as written; the
manual selections remain active for the next one-line Effect.

## Reports

Human document output shows the original numbered source followed by a
`Structure -> Event/Section -> Effect/Condition -> Expression/Type` tree. Each
resolved node can include its registration identity, implementation class,
addon, pattern, return type, multiplicity, default provider, metadata, and
addon-defined public data. Diagnostics and recoveries refer to spans in the
whole submitted source.
On color-capable terminals, the numbered source uses the same syntax-category
colors as one-line Effect output; redirected and JSON output remain plain.
In interpolated strings, the quotes and text remain white, `%` delimiters are
light gray, and each embedded Expression keeps its normal semantic color.

JSON is the machine-readable contract:

- one-shot and one-line Effect reports use `schemaVersion: 7`;
- multiline document reports use `schemaVersion: 1`;
- document nodes form an arena: `roots`, `children`, and node captures contain
  numeric node IDs, avoiding recursively serialized values;
- mapped spans retain every original source origin and macro expansion ID;
- `state` summarizes committed reads and writes for the document revision.

Human formatting may evolve for readability. Consumers should use JSON and
check its report schema independently from the SSG snapshot schema.

## Current Boundary

The parser recognizes only syntax represented by the snapshot, CoreLibrary,
and loaded parser addons. It performs static analysis, not Minecraft runtime
execution. A submitted document can register and use Functions during that
document's two-pass parse, but declarations are not yet accumulated as a
project-wide symbol index across separate submissions. Cross-file variables,
symbols, and server-backed semantic queries remain later LSP work.

## Tests

```console
cargo test -p skript-repl --locked
```

Tests cover one-shot compatibility, Event and Section contexts, multiline
buffering, indentation, paste, explicit submit, cancel, EOF, block comments,
partial trees, nested document hierarchy, default Expressions, JSON reports,
and modern/legacy SSG snapshots.

# Bounded source preparation

WIT **0.48.0 / ABI 30.0** introduced the optional lifecycle capability
`parser.source-preparation` v1 and `catalog-data` v5 JSON array paging. The current
world is **0.49.0 / ABI 31.0**. Rebuild
the host, Core and every addon together. Older ABI/WIT components are rejected;
an ABI 31 component that does not declare the preparation capability is not
called. The preparation export remains part of `addon` and must be implemented.

## Lifecycle and authority

After successful `initialize`, before registration or parsing, the host calls
`addon.prepare-source-step(context)` sequentially. Context contains a host-issued
generation, zero-based step and the exact retained source digest/snapshot ID.
The owner keeps its staging state private and returns `pending(label)`,
`ready(label)` or `unavailable(label)`. Labels are bounded diagnostics, not proof
tokens. A declared unavailable feature may remain conservative while the addon
loads; a trap, forbidden import, cancellation or host quota failure aborts loading.
No partially prepared component is published.

Core and each addon own semantic validation. The host verifies identity,
ownership, lifetime and execution bounds. `ready` does not authenticate a Java
implementation, server registration generation, producer origin, storage result,
or type. Core's registry projection is structural only; its private prepared
projection cannot grant a storage receipt. Missing getter/producer evidence
continues to produce Unknown.

Only immutable source access (`source`, `documents`, `read-document`,
`read-json-records`) and the caller's existing owner/hash-bound `addon-data`
access are allowed during preparation. StateStore, tree/parse queries, dynamic
syntax, semantic publication and context mutation are forbidden. Ignoring an
import error does not hide the attempted violation. This read-only frame may be
discarded on interruption; it does not opt the owner's ordinary hooks into
discard/restart safety.

Preparation generation participates in both invocation and body cache worlds.
A recreated instance prepares afresh and invalidates those caches. Ordinary
document edits retain immutable source preparation but still revalidate current
revision, scope, provenance, State reads and semantic authority. A different
source uses a different host/source lifetime; no old projection is imported.

## Independent limits

| Boundary | Limit |
| --- | --- |
| One guest step | Existing configured fuel (default 20 million) and call deadline |
| One component job | 64 steps; 128 source queries per step |
| Retained host/source preparation time | 5 seconds cumulatively, shared by component jobs and retries |
| Component job selected unique input | 4 MiB, charged once for each retained document/dataset |
| Component job returned payload | 2 MiB cumulatively, including accounted source metadata |
| Preparation linear memory | 16 MiB per linear memory; ordinary configured memory/count limits remain unchanged |
| JSON document | 4 MiB |
| JSON page response | At most 64 KiB; caller may request less |
| JSON raw row / page raw scan | 512 KiB each, even if selected fields are small |
| JSON index cache | 8 source/path keys and 4 MiB logical index/key bytes |
| Host telemetry | Latest 16 successful/explicit-unavailable reports, with dropped-report count |

The time budget includes guest execution and native callbacks and is charged
even on failure. It excludes compilation, instantiation and time between loads.
Remaining time constrains each epoch deadline and is checked again before
accepting Ready. Limits are ceilings, not a promise that every input within the
byte limit can be validated under its step fuel. Quota failures do not refill the
retained-source clock.

The 64 KiB chunk limit is not the total document limit. The SSG registry
observation and Manifest/header bounds remain unchanged (the observation remains
512 KiB). These bounds must not be confused with the whole multi-document
snapshot, unique job input or returned payload. A future larger graph may need
finer semantic validation steps even when paging succeeds.

## Generic JSON pages

`catalog-data.read-json-records(source-digest, snapshot-id, document, path,
fields, offset, limit, max-bytes)` returns the total record count, each original
index and its JSON bytes, and an explicit next offset. `path` is a list of up to
8 literal decoded object keys (128 bytes each), not a JSONPath/filter program.
The source/path index shares immutable source bytes and stores row ranges.

An empty `fields` list returns exact raw row bytes. At most 16 distinct field
names may be selected; projection preserves raw JSON values, including strings,
wide numbers and null, and preserves missing fields as absent. Object keys and
row counts are bounded (4096 and 8192). An ambiguous duplicate key or invalid
structure fails closed. A missing path is distinct from malformed data. Original
row order and indices are retained. A full page may be shortened by response or
raw-scan bounds; an oversized first row is an error, never fake end-of-input.

Digest/snapshot mismatches are rejected before cached rows are used. A cursor
is only an array offset and conveys no authority across another source. The old
v4 `read-record-fields` API remains available; it has no request-specific byte
bound and does not provide nested paging.

## Core use and measurements

Core incrementally validates the Manifest observation arrays, then selected
Types/EventValues fields, detail references and source projection, before
publishing a private structural projection. Staging failure drops the builder.
All old storage proof requirements still apply.

Use `ParserHost::source_preparation_reports()` for bounded per-step fuel, call
time, selected callback time and logical byte/index observations. Fuel excludes
native work; callback time covers instrumented document/page handlers, not every
import. Maximum approved single-memory growth is not aggregate memory, RSS or live heap usage.
Logical raw bytes examined are not physical copies. Compilation, initialization,
full parse and editor response must be measured separately. In particular a
Core guest run against a reflect-containing snapshot is not a measurement of
the reflect guest's own preparation or addon-data route.

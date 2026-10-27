# Live suffix-member editing queries

This optional public host service uses the existing WIT 0.47.0 / ABI 29.0 custom
parser interface. A Component requires `parser.editor-completion` version 1 and
`parser.additional-parse`, together with project-stable semantics. An older host
without the editing capability rejects that manifest. ABI equality alone is not
a sufficient compatibility check; no WIT record layout or production fuel limit
changes for this service.

An opt-in Parser subscription has the route
`editor.completion/<own-component-id>/<one-punctuation-trigger>`, phase `Parser`,
capability `parser.additional-parse`, and mode `Override`. A foreign owner route
is rejected. Registered order is preserved. The current LSP advertises `.` as an
automatic completion trigger in addition to `{`; other trigger routes are only
available through ordinary/manual completion and document reanalysis.

## Query and output

The native parser invokes the service once before each eligible body line's
statement candidate speculation, while its accepted prefix and prepared owner
are still alive. The request contains that exact line input and mapped span.
Additional `host.expression` requests receive the full current native context,
including event classes and active Section frames. The provider may parse
receiver candidates and return editing proposals. Arbitrary source string
matching is not a substitute for receiver/type resolution.

A handled service returns one successful custom parser result with one
`editor-query-result` root, no summary, children, function or diagnostics. Its
single `skript.editor.query-result.v1` metadata record contains:

```json
{
  "version": 1,
  "complete": false,
  "exclusive": true,
  "proposals": [{
    "id": "stable-full-signature-identity",
    "label": "member()",
    "filterText": "member",
    "insertText": "member()",
    "snippet": null,
    "start": 7,
    "end": 7,
    "detail": "Declaration information",
    "documentation": "Source and unresolved conditions"
  }]
}
```

`start` and `end` are UTF-8 byte offsets into the exact line input, not document
or UTF-16 offsets. The LSP validates and converts them for the actual request
position, preserving receiver text and existing call syntax. Current v1 proposals
are exclusive member slots rendered with Method kind; nonexclusive output is
rejected instead of accidentally replacing ordinary completion.

Success here means that an editing query completed. Its result is never adopted
as an Expression, a semantic AST or a type/storage/callability proof. Reserved
editing parser results cannot be requested as ordinary semantic parsers. The LSP
records `queryContextVerified: true` but `contextVerified: false` and
`semanticGuarantee: false`: provider candidate semantics remain unverified.

## Isolation and bounds

StateStore writes, including deletion and compare-and-swap, are denied during the
query. Overlay changes are rolled back even on failure. Read observations,
semantic receipt registries, unavailable-dependency observations and incremental
invocation cache work are quarantined from the adopted parse. Guest purity is an
explicit project-stable service contract; host rollback is not a claim to undo
arbitrary guest memory or I/O. Existing extension faults and cancellation retain
their ordinary failure behavior.

Preparations in a nested query cannot replace or revoke the outer document's
owner attempts. An outer owner rejection revokes descendant batches, unknown
parent recovery does not publish them, and failed/cancelled/stale document results
are not published. Trigger-bearing bodies are not cached without a replayable
editing ledger; unrelated bodies can still reuse. With no service loaded the
normal parser skips editing context/source preparation entirely.

Current limits are 4096 bytes per line, 128 trigger-bearing line query attempts
per document, 256 proposals per result, 128 KiB per metadata record and 2 MiB
retained payload. The client menu has an aggregate 512-item limit across services.
Providers should use a smaller batch and mark truncation/incomplete source data.
Exact single-origin source equality and UTF-8 boundaries are required; generated
or ambiguous source is suppressed. The service is intentionally bounded, not a
promise of arbitrary late-file completion after its query budget is exhausted.

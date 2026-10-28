# Retained registry observations and storage proofs

Core prepares optional SSG `registryCapture` v1 observations from the same retained
source as its catalog records. Manifest reads use 64 KiB chunks, with a 2 MiB total
cap and at most 64 advancing calls. Each chunk checks source identity before and
after, offset, stable total length and byte bounds before allocation. Bytes are
concatenated before UTF-8/JSON decoding. Transport, cancellation and source-change
failures do not create permanent negative cache entries. The Operations document
retains its separate 64 KiB limit.

The whole-Manifest limit includes ordinary environment metadata. It differs from
SSG's 512 KiB limit for the compact observation object itself. Larger or malformed
inputs fail closed; neither limit is removed.

Preparation parses the Manifest once, moves its observation out of the JSON tree,
and checks every Type/EventValue projection, exact binding and referenced identity.
The checked input retains both source/snapshot identity and the exact raw-Manifest
hash. Cache keys include the full retained-source digest, not snapshotId alone.

An observation is not an authenticated `CaptureRegistry`/`ProducerClaim`. Equal
closed registration boundaries do not supply an independent mutation epoch.
Retained getter identity does not prove a hidden implementation or native event
producer origin. Internal blockers distinguish absent/invalid observation,
legacy-profile mismatch, missing generation, getter and producer evidence.

The existing source-audited legacy backend and private context receipts retain
their independent authorization checks. `prepare()` still authorizes receipts
only when that model is available. A successfully projected mixed environment
alone issues no storage result, provenance seal or receipt. This change does not
claim Player storage type recovery or compatibility with stale reflect member
datasets; a mismatched dataset must continue to refuse initialization.

Real isolated Core/reflect captures are covered by native Rust regression data in
`core-library/src/storage/evidence/observation/data/real-capture/`. They test the
read/projection boundary and conservative Unknown outcomes, not runtime producer
authentication. Public RPC/editor acceptance must be reported separately.

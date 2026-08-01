# Matching hook test addon

[日本語](README.ja.md)

A deterministic WASM fixture for `parser-wasm` pattern-matching integration tests. It subscribes to one exact registration, records every matching scope invocation in a private parse-scoped StateStore namespace, and overrides the element scope so otherwise non-matching input succeeds.

The host test runs the same registration as a selected candidate and an alternative. The fixture therefore proves both typed `MatchingPayload` dispatch and candidate-level rollback: only state written while evaluating the selected candidate remains in the parse transaction.

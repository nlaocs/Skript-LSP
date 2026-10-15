# Function leaf fixture

`xtask build-test-components` builds `artifacts/function-addon.wasm` for the
current WIT package/ABI. This is a parser contract fixture, not a Skript runtime.
It never executes a Function or Effect.

The deliberately small `wasm echo(value)`, `wasm log(value)`,
`wasm sum(value; value)`, `wasm named(base; n)`, and `wasm attachment(value)`
grammars attach the actual dummy_echo/log/sum/attachment Function identities from
`utilities/skript-repl/tests/data/current-multi-addon-2.15.4`.
Arguments use `host.expression` requests and verified host-token/root references.
Tests check names, identity pairs, addon metadata, parameter order, nested calls,
omitted/named/plural arguments, return contracts, and CLI/REPL output.

`wasm bad-*(value)` variants deliberately corrupt ownership, omission, names,
kind, references, or spans after a state write. Host tests require a typed fault
and rollback; document tests require an independent later line to survive.
The fixture grammar and identities are not upstream compatibility observations.

`wasm quota-*(value)` expands one Function name, identity, parameter name, or
supplied name beyond the configured output budget. Host tests require output
quota rejection, rollback of nested Function state, and a usable later parse.
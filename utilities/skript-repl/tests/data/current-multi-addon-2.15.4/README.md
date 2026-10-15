# Current nine-plugin parser snapshot

All 21 JSON files are the unmodified complete SSG schema 7 export generated at
2026-10-15T00:32:03.807907299Z, snapshot
`6e9290cfcfdb7e8861a88681d62ea105ba9099e0065a5ba46610e7d431025584`.

Producer: SSG source `f7542b9ba42a54d1eb6478d8ff82ebef7fdb572f`, whose tree equals
main merge `27366cdeff5b120b0cafe4ef21a79ed7788eb8e9`; JAR SHA-256
`1d9fe40725f88ca524e49826efdb8e160e3a7b2bfea9a7ed429253be6e748a14`.
Paper 1.21.11-132 / Java 21.0.3 ran in an isolated Debian environment with no
user scripts. Manifest.json records all nine plugin JAR hashes. The eight other
plugin JARs match the predecessor schema 3 multi-addon profile exactly; only SSG
was updated. No generated metadata, registration order or digest was patched.

This fixture exercises the normal CLI loading/reload contract and the original
join/player inputs, including string/variable interpolation and plural players.
The archived schema 3 and legacy fixtures remain unchanged for explicit library
regression analysis. Runtime collection and focused parse observations do not
certify all syntax, game values or whole upstream object graphs.

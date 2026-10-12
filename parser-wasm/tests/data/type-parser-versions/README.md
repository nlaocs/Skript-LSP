# Type parser version fixtures

Each directory is an unchanged complete SSG `snapshot/` export. Historical
schema 5/6 fixtures contain 20 files; schema 7 adds `BlockData.json` (21 files).
They retain original manifest metadata, language documents, parser classes,
type ordering, version-specific literals and documentation examples.

| Directory | Generator profile | Skript | Minecraft | snapshotId | contentDigest |
| --- | --- | --- | --- | --- | --- |
| `skript-2.15.4` | skript-2.15.4 | `2.15.4` | `1.21.11` | `a57b3b7f9bf384fc356a82c276a97410d76e9518ac8eaf862f35c008ae217a4a` | `462a8bee395453c18baf93d032d8c7e97e9eb2ae0c2b714ae96589115a5dd70a` |
| `skript-2.16.0` | skript-2.16.0 | `2.16.0` | `1.21.11` | `8b164811497583c1d5cefbe0abe2ca8e79a5b7234e17d1c1058e5cdf21608477` | `4e238b3b4b0f32dd22544a9e1d210e88799a1f6ae661ff1ce28b96d936e6a0c4` |
| `skript-2.16.2-java25` | SSG #9 paired Java25 | `2.16.2` | `26.2` | `0e896c28e8d83a5124f1e66f8a71f4cea2bf384e9c611aba3022de8544c11c5f` | `304605a3b5abc14b8fbc9820bc85a8f08be19a1958c6663102eefefdba37e5e5` |
| `skript-2.6.4-mc-1.12.2` | skript-2.6.4 | `2.6.4` | `1.12.2` | `8f8223a8f71a5e73fba2b6f12ebd5a52989b47b41fcdf9be1dc4edbbb9fc48c1` | `fe701419551f649a4f7960571f967faa375b66076ce2ecf72de14d8f13cad8e9` |

The curated REPL corpus loads every fixture through `ssg::load` to validate
digests, snapshot identity, complete inventory and cross-file references. The
three historical versions are also covered by the existing type-parser tests.
Server installations are not a test dependency.

`skript-2.16.2-java25` was exported during the SSG #9 paired converter-collection
verification, using modified Skript 2.16.2, Paper 26.2-121 and Java 25.0.4.1.
It is experimental integration evidence, not a stock 2.16.0 fixture. The merged
SSG code matches the tested fix. Its complete export passed the corpus loader
and documentation-owner checks independently of the prior direct converter
registry/export comparison. See `utilities/skript-repl/tests/corpus/README.md`
for artifact hashes, collection limitations and baseline coverage.

# Type parser version fixtures

Each directory is a byte-for-byte copy of a complete 20-file SSG `snapshot/`
directory produced by the local Generator integration profiles. The fixtures
keep the original schema, manifest metadata, language documents, parser class
values, type ordering, and version-specific type literals.

| Directory | Generator profile | Skript | Minecraft | snapshotId | contentDigest |
| --- | --- | --- | --- | --- | --- |
| `skript-2.6.4-mc-1.12.2` | `skript-2.6.4` | `2.6.4` | `1.12.2` | `8c4b52a721c73f2ea7750da6f7da036644da7d364d4ba90bdd42e1f7fb02a510` | `4cbcc106b772f4a1540527690298fb49ce90a25ec522b56feabd726d4350c5be` |
| `skript-2.15.4` | `skript-2.15.4` | `2.15.4` | `1.21.11` | `e945d534fab4aa2722b603445af9e0f885995b6d436249ec71fe4f37c517532e` | `a2204b2718a6d4bdd2ae54d75e184c16b8d32dfd1486ff50318a8d1e1aa0e4f5` |
| `skript-2.16.0` | `skript-2.16.0` | `2.16.0` | `1.21.11` | `572c548cc2ef5e327f76688d80137a3c9ed4f74de5738b5d627b121886533ed4` | `102e26933d2731c05fb45a7599c28b3e9f56d1e762c5cecbe73a5f2cdab71bdf` |

The test loads every fixture through `ssg::load`, so the loader validates the
manifest digests, snapshot ID, complete file inventory, and cross-file
references. Server installation directories are intentionally not included;
the checked-in snapshots are the only test dependency.

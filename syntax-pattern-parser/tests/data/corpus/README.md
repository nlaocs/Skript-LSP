# Generator corpus fixtures

These directories contain unmodified syntax JSON files produced by
SkriptSyntaxGenerator. The corpus tests parse every registered pattern without
network access.

## dummy-addon-2.15.4

- SSG schema: 3
- Skript: 2.15.4
- Minecraft: 1.21.11
- Addon: SkriptDummyAddon 1.1.0
- Snapshot ID: 7dd71a4fa5db141284ee6eef6161eee9d4d8421dcbaee9d24d387b00bdc012a3
- Source: build/integration/skript-2.15.4/snapshot

## multi-addon-2.15.4

- SSG schema: 3
- Skript: 2.15.4
- Minecraft: 1.21.11
- Addons: SkJson, skript-reflect, SkBee, Lusk, SkriptDummyAddon, Hippo, and
  skript-particle
- Snapshot ID: c0b089bba0b3ece13d2be50c9b6293bbd4f48153a346cff0a4fa75b765c6b5cc
- Source: run/plugins/SkriptSyntaxGenerator

The multi-addon corpus is a complete, unmodified 19-file snapshot shared with
the `ssg` loader tests. Its generated PluralRules.json is used by the pattern
tests. The DummyAddon corpus contains the six syntax files and Manifest needed
by the parser tests, and uses the existing ../PluralRules-2.15.4.json fixture,
which is byte-for-byte identical to that schema 3 snapshot's plural rules.

Do not edit generated JSON by hand. Refresh a corpus from a complete SSG
snapshot. Keep every file listed by Manifest.json together when the fixture is
used by the `ssg` loader.

# BlockData state whitespace oracle

`block-data-state-whitespace.tsv` records Java 21 `Character.isWhitespace` and
Paper 1.21.11 BlockStateParser acceptance for 20 state-token positions. The
codepoints cover Java whitespace/space characters, Unicode Control and Format
characters, and their immediate neighbors: 302 codepoints, 6040 parser cases.
The complete Java Unicode scalar scan found 25 whitespace codepoints.

The third column is a bitmask, with positions ordered as the unit test's
`placements` array. The masks are actual Paper results after the fixed Skript
BlockUtils normalization, including rejection of non-ASCII whitespace after
the final comma. Quoted contents keep all non-ASCII characters.

Generation sources, full matrix and execution evidence are saved in
Skript-project-management/pr-r2-evidence-2026-10-12/.

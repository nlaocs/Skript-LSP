# AST macro テスト addon

Issue #58 の AST macro component-boundary テストで使う決定的な WASM fixture
です。現行 worktree の `nlaocs:skript-parser-addon@0.37.0` と ABI 19 を対象に
しています。

対象 node の `text` によって動作を切り替えます。

| node の text | 動作 |
| --- | --- |
| `delete` | 空の replacement tree を返し、対象 node を削除します。 |
| `one` | 生成した root 1 個で置換します。 |
| `many` | 生成した root 2 個で置換します。 |
| `metadata` | 対象 subtree を保持し、addon 所有の metadata を追加します。 |
| `preserved` | replacement を返さず、`preserved` の node を保持します。 |
| `cycle` | 別の `cycle` node を生成し、host の cycle 検出を発生させます。 |
| `reject` | diagnostic 付きの型付き `Reject` を返します。 |
| `addon-error` | diagnostic 付きの型付き addon error を返します。 |
| `trap` | panic して WASM trap を発生させます。 |
| `state-write` | parse scope の private StateStore に書き込み、node は保持します。 |

生成 node はすべて対象 node の mapped span を流用します。生成 node は
`macro`、保持される node は `preserved` の context-origin を要求します。
また、すべての呼び出しを private な parse-scoped namespace に記録するため、
採用・Reject・addon error・trap における commit/rollback を host 側から検証
できます。

この crate が実装するのは AST macro export だけです。Text macro、Tree macro、
汎用 hook は誤 dispatch が見えるよう `UnsupportedCapability` を返します。

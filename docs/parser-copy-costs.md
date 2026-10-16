# パーサーのコピー費用とHookCallログの世代共有

## 変更と採用範囲

PR #130のmerge後のmain `f7ffc78c362149c14d1b66eba2170a7ab4c0d0a6` を基準に、
既存の実ドキュメント3件で候補探索・checkpoint・memoのコピー費用を測った。
この単位では、非公開の `WasmPatternHooks.calls` と `HookEffectsCheckpoint.calls` を
`Arc<Vec<HookCall>>` に変更した。保存・復元は世代の参照を共有し、非空の追加時だけ
`Arc::make_mut` で独立させる。空のdispatch結果では世代を分離しない。
公開結果とdeferred suffixの `Vec<HookCall>` は維持した。

候補順・採用候補・A→base→B→Aの復元を保ち、同じ長さで内容が異なるログも復元する。
fault観測は引き続き復元対象外で、新しいfailureや既存failureのspan更新を残す。
stateのsavepoint、revision、delta、memo key、キャンセル判定は変更していない。
型・Addon・入力固有の分岐は追加していない。

memoのコピー費用はさらに大きいが、公開mutable node、再帰seed、effectsのidentityに
関わるため別単位に残した。まずprivateなログ共有だけで効果と契約を確認した。
測定値はこの入力・ビルド・環境の結果であり、パーサー全体の計算量を主張しない。

## 条件と観測の範囲

- Debian 13 LXC、i7-8700、可視CPU 4、RAM 16 GiB。Rust 1.97.1。
  buildは `--jobs 1`、実行は `nice -n 10`、計測は `--test-threads=1` で直列。
  CPU affinity・周波数は固定していない。Windows側で重いbuildは行っていない。
- 同じCargo.lock、既存の12 WASM artifactを固定し、再buildしていない。
  CoreLibrary SHA-256は `3a2ae04052e59ac89acfbbe7a8fddf550ead73bc5353883d8f66cb23927309c4`。
  LXCの開始時sourceは542ファイルのmanifestと照合した。最新mainと実行コードは一致し、
  保存済みsourceに残る差は非実行ドキュメント2件とEOLだけだった。
- schema 7の `skript-2.16.2-java25` export、21ファイル、1,802クラス。
  Snapshot IDは `b7c14c3cf6b8864973c14d5cfa773334e9e502013e5c8fae70b3afc9506be04d`。
  Skript 2.16.2、Java 25、Paper 26.2、SkriptとSSGの2 plugins。
- 既存corpusの `doc-message-event`、`short-teleport`、`research-teleport` を
  既存harnessでevent blockに包んだ。同じcaseごとに新しいsessionを作り、同一入力の
  revision 1をfirst、revision 2をwarmとして各1回測る。worker stackは32 MiB。
- timed範囲は `parse_document`。snapshot load・host初期化・report作成・JSON変換は範囲外。
  report時間はCSVに別記している。CLI/LSPプロセス全体の起動時間ではない。
- 無計装の比較は各モード7プロセス、順序はbaseline/candidateを交互に逆転。
  各プロセスで3case×first/warmを測る。各revisionと全モード間で、
  `parseDurationNs` だけを除いたreport JSON全体の一致を確認した。
  checksumだけによる比較ではない。component failureは0、全caseがmatched。

## 変更前のコピー費用

一時RAII probeとparse threadのSystem allocator wrapperで測った長い入力のwarm値。
34領域のexclusive合計はdocumentのinclusive 5,588.450 msと一致する。
inclusive同士は入れ子なので合算しない。計装の負荷を含むため、速度比較には使わない。

| 領域 | 回数 | exclusive ms | allocation要求回数 | 累積要求bytes |
| --- | ---: | ---: | ---: | ---: |
| memo hit のclone | 11,525 | 631.788 | 12,220,701 | 1,045,084,808 |
| memo growth seed のclone | 2,988 | 505.712 | 6,728,244 | 579,762,541 |
| checkpoint calls の保存・復元 | 346,651 | 417.956 | 9,655,891 | 324,718,309 |
| checkpoint effects の保存・復元 | 346,651 | 230.607 | 4,502,877 | 224,451,081 |
| resolved children のclone | 3,446 | 71.903 | 166,803 | 17,412,711 |
| resolved captures のclone | 3,446 | 14.681 | 268,396 | 25,987,056 |
| 選択nodeのclone | 8,371 | 13.822 | 276,722 | 20,213,147 |

checkpoint callsの保存・復元は累積4,749,300要素をclone対象としていた。
memo hitは604,557 node、growth seedは328,242 nodeのcloneを観測した。
このnode数にはchildren・function arguments・routed captureへの再帰を含む。
node cloneの最大深さはこの観測全体で5だった。

論理inline bytesは要素数×Rust型サイズ、counted string bytesは計測対象文字列の長さの和。
ExpressionNode、ExpressionCandidate、PrefixCandidatesは埋込み領域が重複するため、
種類を横断した総コピーbytesには合算できない。node文字列の集計はmetadata、public data、
return typeなど明記したフィールドに限り、すべてのheap payloadを網羅しない。
HookCallでは2つのString payloadを数えている。Vec::clone_fromは領域を再利用するため、
論理bytesとallocator要求bytesも別の量である。

全Rust allocation要求は 45,979,505 回、
累積要求 3,581,522,857 bytesだった。
これはlive heapやRSSではなく、reallocは新しい全layoutサイズで数える。
他thread、C allocator、guest linear heapの操作は網羅しない。
全量にはregion外の6要求も含むため、region別allocation合計とは区別する。

## 無計装の前後比較

同じtest profile（opt-level=1、line-tables-only、debug assertionsとoverflow checks有効）。
release profileの過去の値とは比較しない。中央値 [最小, 最大]、単位ms。

| 入力 | revision | 変更前 | 変更後 | 中央値の変化 | 改善した組 |
| --- | --- | ---: | ---: | ---: | ---: |
| doc-message-event | first | 88.316 [85.921, 91.694] | 87.469 [85.608, 93.073] | -0.96% | 3/7 |
| doc-message-event | warm | 58.462 [55.538, 61.897] | 56.822 [54.078, 67.964] | -2.80% | 4/7 |
| short-teleport | first | 355.642 [342.077, 412.436] | 343.789 [339.298, 368.734] | -3.33% | 5/7 |
| short-teleport | warm | 352.359 [343.672, 411.300] | 344.774 [340.916, 362.116] | -2.15% | 5/7 |
| research-teleport | first | 4,946.458 [4,882.468, 5,008.656] | 4,484.592 [4,458.999, 4,745.370] | -9.34% | 7/7 |
| research-teleport | warm | 5,031.959 [4,882.845, 5,150.293] | 4,540.217 [4,465.291, 4,597.589] | -9.77% | 7/7 |

長い入力はfirst・warmとも7組すべてで改善した。小さい入力は分布が重なるため、
小幅な中央値差を一律の速度向上とは扱わない。7プロセスの結果であり、
多様なproduction workloadや将来の性能を保証する統計ではない。
全84 timed parseで同一caseの観測JSONが一致した。

## 共有後に残るコピー

別の一時計装runで、保存・復元に加え、書き込み時と公開Vecへの変換時も測った。
こちらは36領域なので、34領域の変更前probeとの時間差を速度改善の根拠にしない。
全3caseのfirst/warm JSONも無計装runと一致した。

長い入力のwarmでは非空追加 12,284 回のうち、共有世代の分離は
12,279 回、clone対象 164,026 要素だった。
その領域のallocator要求は 364,890 回、
累積要求 31,813,723 bytes。
公開変換は 1 回、shared時のfallback cloneは 0 回、
clone対象 0 要素だった。
内部checkpointが残る場合もあるため、公開変換が常にzero-copyになるとはしていない。
ここで前後比較している要素数はcheckpointに関連する全prefixログのclone対象である。
deferred suffixなどに従来からある局所的なcloneはこの要素カウンタには含めず、
次のallocator全量には含めている。

全Rust allocation要求は 36,688,500 回、
累積要求 3,288,545,783 bytesとなった。
減少は要求回数 20.21%、
累積要求bytes 8.18%。
capture/restoreだけを差し引かず、追加時・公開時・Arcの新規割り当ても含む全量である。

snapshot時に観測した単一世代の最大lenは 33、capacityは
64、その時点の既存strong owner数の最大は 95。
これらは全世代を合わせたpeak保持量ではない。Arcは古い世代を最後の参照まで保持するため、
並列session、大きなログ、live heap・RSSのpeakは別途測定が必要である。
共有CSVの `clone_required_operations` はwrite/exportでコピーを要した回数を表す。
snapshot行の0はHookCall要素のcloneがないことを表し、世代共有がないという意味ではない。

## 回帰と独立査読

新しい5テストは、同長・短い・長い他候補を経由した保存世代の復元、復元後追加の独立性、
空dispatchの非分離、callsの一度だけの移動、fault markerとspan観測保持、
deferred suffixの二重適用抑止とrollback後の再適用、owned返却Vecの独立性を確認した。
初回は追加テストの `HookDecision::Continue` が存在せずコンパイル失敗したため、
`ContinueProcessing` に修正した。初回ログも保全している。

4crate（skript-parser / parser-wasm / skript-repl / skript-lsp）のall-features回帰は
670 passed / 0 failed / 7 ignored、46 groups。
既存の候補選択、default、state、fault、SourceMap、native cancellationを含む。
ignoredのうち今回対象の手動計測は比較・詳細計測で別途実行した。rustfmtとdiff whitespaceも確認した。
製品差分と計測を別々の独立査読者が確認した。

同入力の等価性は既存の観測仕様に対する確認である。
既知のFunctionCall argument bindingが最終AST投影で欠ける別問題は修正していない。
キャンセル機構、SSG収集失敗検出、LSP接続の変更も含めない。

## 再現と証拠

既存harnessのSHA-256は `82fe9dde9927ce3304aedff1bb8b15f7efb8deb13a667b4528509b41665dc425`。
基準・候補を同じLXC、wrapper、lockfile、artifactでそれぞれ以下のコマンドからbuildし、
Cargo JSONのprofile/features/fresh=falseと実行ファイルを確認して凍結した。

```sh
./run.sh candidate test -p skript-repl --lib --all-features --locked --offline --no-run --message-format=json --jobs 1
nice -n 10 python3 candidate/utilities/skript-repl/tests/measurements/run_capture_pipeline.py \
  copy-baseline-test.bin copy-candidate-test.bin comparison --samples 7 --candidate-label candidate
./run.sh candidate test -p skript-parser -p parser-wasm -p skript-repl -p skript-lsp \
  --all-features --locked --offline --jobs 1 -- --test-threads=2
```

| 実行ファイル | SHA-256 |
| --- | --- |
| 無計装 baseline | `deab6b4641cf8d7b4fcf0d2ebf27cb9010bf0d560d2dc7396213c0c7d952864c` |
| 無計装 candidate | `8994ea20bb2d88b1c1d0e0278f13fcad603b0eb983c55d8574159dc5e273f580` |
| 詳細計装 baseline | `b14979891897f61e5303fc5b53d09cee3513d65d6acb722349de8c3986e74c1f` |
| 詳細計装 candidate | `643b3a95d8b129276a8c743a90a93aec95d24c1e2fbbdc8ef01082ffacd90e7c` |

- [全84サンプル](parser-copy-costs-samples.csv)
- [全region](parser-copy-costs-regions.csv)
- [種類別clone量](parser-copy-costs-copies.csv)
- [共有・分離・公開変換](parser-copy-costs-sharing.csv)
- [全allocator要求](parser-copy-costs-allocations.csv)

生ログ、全観測JSON、Cargo JSON、SSH実行wrapper、一時probeのsource tarと適用前後SHA、
集計スクリプト、失敗ログは作業場所の `parser-copy-costs-evidence.tar.gz` に保全する。
probe適用前後にソースを照合し、元のbytesへ復元してから回帰を実行した。
12 WASMと以前の12凍結実行ファイル、新しい4凍結実行ファイルのSHAも再確認した。
一時probeは製品の差分に含まない。

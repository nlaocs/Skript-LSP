# HookCallログ共有のrelease・メモリ・保持寿命検証

## 変更と保持契約

基準はmain `f7ffc78c362149c14d1b66eba2170a7ab4c0d0a6`。
ログ共有を導入した `023804d76eb4d9ee2b2329b49e57c5efed10b484` を測定し、
書き込みで分離したVecの未使用容量も旧世代ごと保持される条件を確認した。
最終候補は、共有世代への非空追加時に `Arc::make_mut` した後、追加分を
`reserve_exact` で確保する。単独所有時の追加は従来のVec成長を維持する。
strong countは予約方針のヒントであり、所有権の処理は `make_mut` が担う。
capacityの厳密一致を契約にはしない。

旧世代の保持期間は所有者で決まる。branch restoreだけでは保存済みcheckpointを破棄せず、
matcher frame終了時に対応するbranch/scopeを切り詰める。外へ渡したcandidate stateと
そのcloneも所有者なので、環境の破棄後も世代を保持できる。
正常cancelではrollback成功後にbaseへ戻してbranch/scopeを解放する。
rollbackの後始末が失敗した場合はcleanupエラーを優先し、残る世代は環境破棄時に解放する。
これらの処理とpublicな返却Vecの契約は変更しない。

## 測定条件と3つの範囲

Debian 13 LXC、i7-8700、可視CPU 4、RAM 16 GiB、Rust 1.97.1で直列実行した。
buildは `--release --all-features --locked --offline --jobs 1`、実行は `nice -n 10`。
通常release profile（opt-level=3、debug assertions/overflow checks無効）で、
元のtest profile（opt-level=1）との速度差は比較しない。
CPU affinity・周波数は固定していない。重いWindows buildは行っていない。

Cargo.lock、既存12 WASM、schema 7の `skript-2.16.2-java25` exportを固定した。
Snapshot IDは `b7c14c3cf6b8864973c14d5cfa773334e9e502013e5c8fae70b3afc9506be04d`、
CoreLibrary SHA-256は `3a2ae04052e59ac89acfbbe7a8fddf550ead73bc5353883d8f66cb23927309c4`。
入力3件、event wrapper、32 MiB worker stack、caseごとの新規session、
first revision 1／warm revision 2は[元の測定](parser-copy-costs.md)と同じである。

| 対象 | 測定範囲 | 標本数と計装 |
| --- | --- | --- |
| 速度 | `parse_document`。session作成・report・JSON変換は範囲外 | 各mode 7プロセス、各3入力×first/warm、計84 parse。元harnessで無計装 |
| peak RSS | 入力1件のプロセス全体。setup、first/warm、report、JSON変換を含む | 各入力・各mode 7プロセス、計42。case selectorだけを追加し、Linux `wait4.ru_maxrss`（KiB）を回収 |
| Rust生存要求量 | 全threadのSystem allocator通知から集計。parse開始前値・窓内peak・終了値 | 各mode 3プロセス、計36 parse window。専用allocator probeであり速度には使わない |

各cohortはbaseline/candidateの順序を交互に逆転して再実行した。
baselineの凍結バイナリを最終比較でも使い、最終candidateは3種を新規buildした。
Cargo JSONのfresh=false、profile/features、harness bytes、binary SHAを照合した。
全mode・全revisionのreport JSONを `parseDurationNs` だけ除いて比較する。
RSSはfirstとwarmを分離した値ではなく、速度の14プロセスとも別標本である。
RSSはhigh-water markなので、drop後の解放判定には使わない。

生存要求量は最初のRust allocationから常時計数し、window開始時にliveをゼロへ戻さない。
alloc/alloc_zeroed/realloc成功だけを加算し、deallocとrealloc縮小を減算する。
counter更新とsnapshotを同じ短いlockで保護し、overflow/underflowを検出する。
System操作後に通知を集計するため、並行実行時は通知の順序に従うrequested-layout計数peakである。
C allocator、allocatorの管理領域、mmap、guest linear heapを網羅した物理heap peakではない。
Rust要求量とRSSは合算しない。単独の既知サイズ検査で、開始前allocationの解放、
zeroed、reallocの拡大・縮小、開始値を超えるpeak更新と解放後のpeak保持を確認する。
失敗reallocをOOMで強制する検査は行わない。

## 旧世代を保持する固定条件

実際の非公開checkpoint・追加・復元を一時helperから呼び、128件のprefix、64保存世代、
1件あたり2つのString（各32／1,024 bytes）を使った。shared-prefixは保存間の書き込みなし、
divergentは各保存前に1件を追加する。各条件・各mode 3プロセスを測る。
checkpointを1個cloneして残し、最初の保存へ復元して追加した後、公開Vecへの変換時にも
checkpointを残してfallback cloneを通す。保存集合、残したcheckpoint、公開Vecを順に解放する。

各stageでログ内容・順序をassertし、保持する値を `black_box` に渡す。
8 stageのsnapshotは固定長stack配列へ保存し、計測終了後に整形・出力する。
全所有者を解放したstageとhelper復帰後の生存要求量が開始値に戻ることをassertする。
このstressはHookEffectsCheckpointの復元を使う。実transaction rollback／cancelの寿命は
別の回帰テストで検証し、このstressがその全処理を再現したとは扱わない。

## releaseの速度

最終候補の中央値 [最小, 最大]、単位ms。

| 入力 | revision | 基準 | 最終候補 | 中央値の変化 | 改善した組 |
| --- | --- | ---: | ---: | ---: | ---: |
| doc-message-event | first | 61.027 [60.508, 66.014] | 62.860 [62.258, 68.307] | +3.00% | 3/7 |
| doc-message-event | warm | 46.591 [45.211, 50.457] | 46.971 [45.721, 51.486] | +0.81% | 3/7 |
| short-teleport | first | 281.926 [272.925, 302.992] | 281.343 [271.321, 296.180] | -0.21% | 3/7 |
| short-teleport | warm | 279.379 [266.757, 283.319] | 272.859 [267.790, 284.135] | -2.33% | 2/7 |
| research-teleport | first | 4,034.300 [3,945.192, 4,151.241] | 3,657.384 [3,625.313, 3,744.791] | -9.34% | 7/7 |
| research-teleport | warm | 4,093.508 [4,063.276, 4,219.197] | 3,688.582 [3,607.010, 3,823.263] | -9.89% | 7/7 |

長い入力はfirst/warmとも7組すべてで短縮した。小さい2入力は分布が重なり、
中央値の小差を一律の速度改善とは扱わない。全84 parseの同一caseの観測JSONは一致した。
容量調整前のreleaseも長いwarmで -9.00%だったが、別標本なので調整だけの差とは断定しない。

## Rust要求量と保持stressの結果

実文書のparse window内peak、単位bytes。各3観測の中央値で、この6条件では各mode内の
3観測値が同じだった。開始値・終了値・process全体peakもCSVに残した。

| 入力 | revision | 基準 | 最終候補 |
| --- | --- | ---: | ---: |
| doc-message-event | first | 50,589,987 | 50,583,625 |
| doc-message-event | warm | 50,795,123 | 50,788,761 |
| short-teleport | first | 52,771,789 | 52,767,758 |
| short-teleport | warm | 53,050,572 | 53,046,541 |
| research-teleport | first | 99,182,837 | 99,172,435 |
| research-teleport | warm | 99,705,406 | 99,695,004 |

差は4,031〜10,402 bytesの減少にとどまる。この実文書ではRust要求量のpeak削減は小幅であり、
速度短縮や以前の累積allocation要求削減率を、そのままpeak削減率とは扱わない。

保持stress全stageを通じた開始値からの増分peak、単位bytes。各3観測の中央値で、
各条件・各mode内の3観測は同じだった。容量調整前のcandidateも保全し、混同を避けるため併記する。

| 保存間の書き込み | 各String長 | 基準（owned checkpoint） | 共有・容量調整前 | 最終候補 |
| --- | ---: | ---: | ---: | ---: |
| なし（shared-prefix） | 32 | 976,048 | 57,600 | 51,504 |
| なし（shared-prefix） | 1,024 | 17,994,800 | 823,424 | 817,328 |
| 毎回1件（divergent） | 32 | 1,210,080 | 1,683,432 | 1,196,472 |
| 毎回1件（divergent） | 1,024 | 22,375,392 | 22,578,920 | 22,091,960 |

短いStringでdivergentの世代を残す条件は、容量調整前に基準比39.1%増加した。
共有Vecをcloneした直後の追加で余剰容量を確保し、それも各世代で保持する影響と整合する。
最終候補ではこの条件のpeakが基準を1.12%下回った。共有・分離の契約を変えず、
COW時だけ追加容量の確保方法を変えて、全cohortを再実行した結果である。
各mode計12 windowすべてで、logs-droppedとhelper復帰後のliveが開始値へ戻った。
これはこの固定helperでのRust要求量の確認であり、native heap全体の解放保証ではない。

容量増大の機構は、同じ2 String構造（inline 48 bytes）を使う標準Vec/Arc操作の独立再現でも確認した。
この実行では、最初のunique追加後のlen/capacityは129/256のまま、次のCOW世代は
容量調整前が130/258、最終方針が130/130だった。64番目も192/382対192/192となった。
これは標準コンテナの再現結果で、製品parseの世代別capacityの直接観測とは区別する。

## プロセス全体のpeak RSS

最終cohortの中央値 [最小, 最大]、単位KiB。

| 入力 | 基準 | 最終候補 |
| --- | ---: | ---: |
| doc-message-event | 317,984 [315,740, 319,168] | 317,768 [315,324, 318,728] |
| short-teleport | 317,740 [314,652, 320,264] | 318,256 [317,260, 318,740] |
| research-teleport | 318,060 [315,836, 318,980] | 317,920 [316,916, 319,480] |

全42プロセスで同一caseのJSONが一致した。RSSの分布は重なり、中央値も小さな増減にとどまる。
この測定からRSSの一律削減は主張しない。単独allocator校正3回では、既知サイズの増減に加え、
開始値を1,024 bytes超えたpeak更新と解放後のpeak保持、realloc縮小による3,072 bytes減少を確認した。

## 回帰と制約

追加6テストは、48旧世代と最後の所有者、branch restoreからframe終了までの保持、
環境破棄後のexported candidate state、正常cancelとrollback後始末失敗時の寿命、
共有COW後の連続unique追加、Weakだけを残した追加を確認する。
既存5テストと合わせ、ログ順序・分岐独立性・fault観測・deferred再適用・public Vec独立性を検証する。
初回は追加テストが未実装の `PluralRules::default` を要求してcompileに失敗したため、
既存の2.15.4 fixtureから生成する形へ修正した。初回ログも保全した。
測定後の変更はこのtest module内だけで、製品host・harnessは測定時と同じである。
最後のrustfmt整形も空白だけの差として保存・照合した。
LXCの最終ソース記録543件をローカルと照合し、実行コードは改行差を除いて一致した。
元の転送snapshotにある非実行文書2件の既存差分は、例外記録とともに再現資料へ残した。
修正後のLXC release焦点テストは11 passed / 0 failed / 0 ignoredだった。
全workspaceのrelease・all-features回帰は1209 passed / 0 failed / 7 ignored、84 groups。
unit・integration・doctestを含み、焦点11件をこの集計へ重ねて加算していない。
今回対象の手動計測は別cohortで実行した。rustfmtとdiff whitespaceも確認した。
この検証は固定済みWASMを使うLinux release実行で、CIのtest profileや他OSの結果とは区別する。

参照化・共有の等価性は既存の観測仕様に対する確認である。
既知のFunctionCall argument bindingが最終AST投影で欠ける別問題を修正・証明したものではない。
入力3件と固定した保持stressは、並列sessionや任意Addon・全入力のpeak上限を保証しない。
古いcandidate stateを保持すればそのログも保持し、世代数に依存する費用が残る。
パーサー全体のO(n)化、peakの一律削減は主張しない。

## 再現資料

基準・候補は同じwrapper・lockfile・artifactから次のコマンドでbuildする。
RSS用はcase selectorだけ、Rust要求量用は同一の一時allocator/helperを加え、
計装ありの時間を速度比較へ混ぜない。

```sh
./run.sh candidate test -p skript-repl --lib --release --all-features --locked --offline --no-run --message-format=json --jobs 1
nice -n 10 python3 candidate/utilities/skript-repl/tests/measurements/run_capture_pipeline.py baseline.bin candidate.bin comparison --samples 7 --candidate-label candidate
./run.sh candidate test -p parser-wasm --lib --release --all-features --locked --offline --jobs 1 host::checkpoint_tests -- --test-threads=2
./run.sh candidate test --workspace --release --all-features --locked --offline --jobs 1 -- --test-threads=2
```

| 凍結実行ファイル | SHA-256 |
| --- | --- |
| 速度・基準 | `2030cfa48e41f36a35607c34697bf499a53aa1a447eece437425ba4c17ae6cca` |
| 速度・最終候補 | `3ea941736d255bf8e1d969a6a6b746622b04bfac8cb5836a0d42776725077be3` |
| Rust要求量・基準 | `2ea772e407ce06e214e84726a7b6b1912b1fd162d23e4b8436de72229f1e6587` |
| Rust要求量・最終候補 | `acb06170a864b6ee90f5dca5bc3c24375785a8eec123a1632cdd94936b1ece58` |
| RSS・基準 | `2c91f5967a3e6a4f18c7f222ca4d017139dc9b0555a86d703415a160c0bf7bca` |
| RSS・最終候補 | `d182f82690f83378e15d7b34889342c27d9758db049c089078edd3fc2a304f5a` |

- [releaseの全84サンプル](parser-copy-costs-release-samples.csv)
- [peak RSSの全42プロセス](parser-copy-costs-rss.csv)
- [Rust要求量の全36 window](parser-copy-costs-live-memory.csv)
- [保持stressの全216 stage](parser-copy-costs-retention.csv)

生ログ、全観測JSON、Cargo JSON、凍結binary SHA、固定12 WASM、適用・復元manifest、全一時source、
wrapper、集計・検証スクリプト、失敗ログを `parser-copy-costs-final-evidence.tar.gz` へ保全する。
製品ソースへ一時probeを残さず、最終ソースと12 WASM、全凍結バイナリをSHAで再確認する。

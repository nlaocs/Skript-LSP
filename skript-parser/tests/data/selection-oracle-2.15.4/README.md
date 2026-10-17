# Expression 候補選択の本家比較

Skript 2.15.4 の実際の `SkriptParser.parseExpression` 戻り値を、同じ JVM 起動中に生成した完全な SSG Snapshot と対応づけた fixture。Rust の結果を期待値として採用していない。

Snapshot ID: `61e05eef5d46d8991e4c50bed34c13fefd0acb1eea7f3138d35c0845feb80de1`。
採取日は 2026-10-17 UTC。Paper 1.21.11、Java 21.0.3。全 9 plugin の版・JAR hash・load order は `snapshot/Manifest.json` と `registry.json` に保存している。

## 観測と推論

|情報|根拠・保証の範囲|
|---|---|
|選択 class、return type、isSingle|`observed.json` の実際の返却インスタンス。trace 上の成功候補ではない|
|registry 順、登録 pattern 全体、宣言 return type|同じ boot の公開 registry API。`registry.json` に全 1056 Expression を保存|
|definition/registration ID、pattern index|返却 class と exact literal を全登録から一意に対応づけた推論。matchedPattern 自体はインスタンスに保持されず、観測値は null|
|下位 matcher の候補全体と順序|保存された registry 順で literal に一致する全登録。Expression の型/Event 適用性を検査する前の native matcher 契約。本家が全 alternatives を返したという主張ではない|

同じ class を複数回登録しているため、class 名の一致だけでは合格にしない。最終選択では一意な登録への対応、完全な ID、元の pattern index、消費範囲、型、cardinality を確認し、下位 matcher では順序付きの全候補を確認する。Expression parser は最初の意味解析成功で登録探索を止めるため、ExpressionMatches の alternatives を全登録候補と解釈しない。raw 観測と整理後の期待値も照合する。

## ケース

|入力・条件|目的|
|---|---|
|`selection contract rank` / `reverse`|異なる priority の競合と、その反転|
|`selection contract tie`|同じ priority で競合した登録。採取 boot では ReRegistered が先。クラス名順や登録時刻順を再計算しない|
|`selection contract pattern`|最初の登録の 3 pattern のうち index 1 を選択。候補絞り込み後も番号を保持|
|`selection contract typed`、期待 Object / DummyValue|広い期待型では最初の String、DummyValue では前方の非互換 String を除外。型の具体性による勝者ソートを定義しない|
|`selection contract event`、join / quit|join 専用候補の許可と、除外後の次候補の選択|
|`dummy supplier-backed expression`|既存の実登録と custom supplier。元の pattern index 1|

既存 DummyAddon の実装を公開 registry API で別 pattern として追加した、制御可能な競合である。plugin は全てロードしているが、異なる実 addon 同士の全構文競合、実行値、postLoad、ゲーム内動作、全バージョン互換を証明しない。Rust 側は完全な同一 Snapshot と `NoopExpressionEnvironment` を使う。対象クラスの init は成功する既存 fixture であり、addon provider 全般の意味解析を代行しない。

## 保存ファイルと再採取

- `snapshot/`: SSG が生成した 21 JSON を無変更で保存。部分抽出・手並べ替えをしない。
- `snapshot-files.json`: 各 raw ファイルの採取時 SHA-256。
- `observed.json`: 本家への入力、期待型、Event classes、parse mode と実戻り値。
- `config.json`: 追加登録と入力の一覧。
- `expectations.json`: 観測を一意に対応づけた identity と下位 matcher の候補列。
- `provenance.json`: 公式 release、Paper remap、観測コード、および保全確認。
- `fixture-bytecode.txt`: 実測と同じ hash の DummyAddon JAR の javap 出力。対象 init の固定 true と Event 制約を確認。
- `observer/jp/nlaocs/skript/selectionobserver/`: バイトコードを書き換えない attach agent と観測コード。本家 Skript の Expression 選択結果を観測する。
- `observer/agent.mf`: 再採取用 agent の manifest。
- `observer/source-manifest.json`: 採取時と改名後のソースの SHA-256、および変更範囲。

公式 JAR は [Skript 2.15.4 release](https://github.com/SkriptLang/Skript/releases/tag/2.15.4) の SHA-256 `c59854d869562e822a182a5ea66219030718a213d53b67182e5d9bdecb15865d` と一致した。採取サーバーは Paper remap 後の JAR。保存済み Paper index が元 digest と remap JAR を結び、remap 後 hash は `277393a965e164e854f5280aa1c30ce92e69b3fab4df5063711743f9d7847327`。Skript のソース・JAR を独自改変していない。

再採取は同じ版・hash の plugin 群と Java/Paper を専用コピーで起動する。プレイヤーがいないこと、Skript script が空であることを確認し、既存サーバーは操作しない。Java 21 の `javac` に既存の Skript/Paper/Gson/Adventure API の classpath と `--add-modules jdk.attach,jdk.httpserver` を渡して observer をコンパイルする。fixture のディレクトリを基準として、新しい外部作業ディレクトリの絶対パス `OBSERVER_BUILD`、既存 API の `OBSERVER_CLASSPATH`、専用 JVM の `OBSERVER_PID` を指定する。

```sh
mkdir -p "$OBSERVER_BUILD/classes"
javac --release 21 --add-modules jdk.attach,jdk.httpserver \
  -cp "$OBSERVER_CLASSPATH" -d "$OBSERVER_BUILD/classes" \
  observer/jp/nlaocs/skript/selectionobserver/*.java
jar --create --file "$OBSERVER_BUILD/observer.jar" \
  --manifest observer/agent.mf -C "$OBSERVER_BUILD/classes" .
java --add-modules jdk.attach -cp "$OBSERVER_BUILD/observer.jar" \
  jp.nlaocs.skript.selectionobserver.SelectionAttach "$OBSERVER_PID" "$OBSERVER_BUILD/observer.jar"
```

manifest の `Agent-Class` は `jp.nlaocs.skript.selectionobserver.SelectionAgent`。HTTP は loopback のみ、既定 port は 18124。

保存済みの観測は、改名前の package で採取した履歴である。`provenance.json` の `observerSha256` はそのとき使用した **JAR** の hash であり、改名後のソースや再ビルドした JAR の hash ではない。raw 観測、Snapshot、採取時の hash は変更していない。`observer/source-manifest.json` から、採取に使用した旧ソースを保存した commit をたどれる。改名後の変更は package 宣言と反射で読むクラス名のみで、観測処理は同じだが、新しい JAR での再採取は行っていない。ソースと manifest の改行は `.gitattributes` で LF に固定している。JAR は ZIP entry の時刻等でも bytes が変わるため、再ビルド時に採取済み JAR と同じ hash になるとは限らない。

同じ boot で `/registry`、`/configure`（config 全体）、各 `/parse`（config の case）、`/export` の順に POST する。`/export` は通常の `skgen` を同期 dispatch する。integration 自動終了モードは使わない。export command 成功、generatedAt 更新、schema 7・21 files、全 Expression の order/class/patterns/returnType と plugin の版/hash/order が registry と完全一致することを確認する。export 前後の registry も一致させる。採取後は自分が開始した専用 JVM だけを終了し、コピー元の設定・JAR hash 不変を確認する。

新しい JVM では同順位の hash 順が変わり得る。既存 Snapshot の順や identity を手で修正せず、raw 観測と完全な Snapshot を一緒に再採取する。native 結果に合わせて観測値を書き換えない。

参照した本家ソースは tag `2.15.4`（tree `702e3484bc5ab533fc66a039c81f4e8909767571`）の [SkriptParser.java](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/ch/njol/skript/lang/SkriptParser.java#L202) と [SyntaxRegister.java](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/org/skriptlang/skript/registration/SyntaxRegister.java#L47)。前者は registry の順で型変換可能性を確認し、最初の成功要素を返す。後者の tie の挙動を native で再実装せず、SSG の解決済み順を使用する。

# dynamic-syntax-addon

[English](README.md)

`dynamic-syntax-addon`は`parser-wasm`用のtest専用WebAssembly Componentです。
production addonではなく、LSP binaryにも埋め込みません。

Rust registryを直接呼ぶだけでは十分に検証できない動作について、guest実装を含む
end-to-end testを提供します。

## 検証する動作

`addon.initialize`中に次を行います。

- `parser.hooks`と`parser.dynamic-syntax`を要求する
- `initial-effect`というdynamic Effectを登録する
- legacy fixture内のstatic Delay Effectをdefinition IDでoverrideする

Document prepass hookでは次を行います。

- 以前の`prepass-effect`を削除する
- document固有のdynamic Effectを登録する
- `initial-effect`より後という順序制約を設定する
- document textが正確に`reject`の場合、typed rejectionを返す

rejection経路により、dynamic registry writeがparser transactionと同時にrollbackされることを
検証します。host integration testではfreezeとcomponent unloadも検証します。

Delay definition IDは、commit済みのMinecraft 1.12.2 / Skript 2.6.4 SSG fixtureへ意図的に
結び付けられています。これはtest dataであり、applicationが流用できる安定IDではありません。

## ABI shape

crateは`../../parser-wasm/wit`からguest bindingを直接生成し、`parser-addon` worldで必要な
すべてのexportを実装します。

hookは実装されています。text、tree、AST macro exportは`unsupported-capability`を返します。
このfixtureはmacro実行ではなくdynamic syntaxを対象にしているためです。

CoreLibraryと同様、`parser-wasm`へ`default-features = false`で依存します。Wasmtimeをlink
せず、guestから互換性定数を再利用します。

## Build

workspace taskを使用します。

```sh
rustup target add wasm32-unknown-unknown
cargo run -p xtask --locked -- build-test-components
```

出力:

```text
artifacts/dynamic-syntax-addon.wasm
```

生成componentはcommitしません。`xtask`はartifactを配置する前に、完全なparser-addon
interface setをexportしていることを検証します。

## テスト

native compileでguest bindingを確認します。

```sh
cargo test -p dynamic-syntax-addon --locked
```

実際のlifecycle assertionはnative Wasmtime host上で実行します。

```sh
cargo run -p xtask --locked -- build-core-library
cargo run -p xtask --locked -- build-test-components
cargo test -p parser-wasm --test dynamic_syntax --locked
```

WIT worldを変更した場合は、workspace testより先にCoreLibraryとこのfixtureを両方buildし直して
ください。

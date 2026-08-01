# Effect Hook Test Addon

[English](README.md)

`effect-addon`は型付きEffect lifecycle ABI用のtest専用WebAssembly Componentです。Effect
categoryとDummyAddonのexact registration 2件へsubscribeします。一方は候補metadataを置換し、
もう一方は一致したEffectをdiagnostic付きでRejectします。

すべての経路がParse scopeのprivate stateへ書き込みます。host integration testでは、採用された
置換stateだけが残り、Rejectまたはunknown Effectのstateがnested Expressionの処理と一緒に
rollbackされることを確認します。

他のfixtureとまとめてbuildします。

```sh
cargo run -p xtask --locked -- build-test-components
```

生成される`artifacts/effect-addon.wasm`はcommitしません。
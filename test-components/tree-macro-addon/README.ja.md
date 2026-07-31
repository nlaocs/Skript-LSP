# Tree macroテストAddon

parser-wasmのTree macro統合テストで使う決定的なWASM fixtureです。ノード削除、1対多置換、Section本文の保持/置換、再帰展開、cycle検出、型付きreject、trap、不正なedit、StateStore書き込みのtransactionを検証します。
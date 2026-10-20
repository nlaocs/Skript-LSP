import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { Registry, parseRawGrammar, INITIAL } from 'vscode-textmate';
import { loadWASM, OnigScanner, OnigString } from 'vscode-oniguruma';

test('documentation grammars color pattern slots and example interpolation without syntax names', async () => {
  const wasm = fs.readFileSync(require.resolve('vscode-oniguruma/release/onig.wasm'));
  await loadWASM(wasm);
  const registry = new Registry({
    onigLib: Promise.resolve({ createOnigScanner: sources => new OnigScanner(sources), createOnigString: value => new OnigString(value) }),
    loadGrammar: async scope => {
      const file = path.join(__dirname, '../..', 'syntaxes', `${scope.slice('source.'.length)}.tmLanguage.json`);
      return parseRawGrammar(fs.readFileSync(file, 'utf8'), file);
    },
  });
  try {
    const pattern = await registry.loadGrammar('source.skript-pattern');
    const tokens = pattern!.tokenizeLine('send %objects% [to %players%]', INITIAL).tokens;
    assert.ok(tokens.some(token => token.scopes.includes('variable.parameter.skript-pattern')));
    assert.ok(tokens.some(token => token.scopes.includes('punctuation.definition.group.skript-pattern')));
    const example = await registry.loadGrammar('source.skript');
    const line = 'send "hello %{_name}%" # greeting';
    const exampleTokens = example!.tokenizeLine(line, INITIAL).tokens;
    assert.ok(exampleTokens.some(token => token.scopes.includes('variable.other.skript')));
    assert.ok(exampleTokens.some(token => token.scopes.includes('punctuation.section.interpolation.begin.skript')));
    assert.ok(exampleTokens.some(token => token.scopes.includes('comment.line.number-sign.skript')));
    const escaped = 'send "say ""# hello""" # outside';
    const escapedTokens = example!.tokenizeLine(escaped, INITIAL).tokens;
    assert.ok(escapedTokens.filter(token => token.scopes.includes('comment.line.number-sign.skript')).every(token => token.startIndex === escaped.lastIndexOf('#')));
    const partial = example!.tokenizeLine('send "unfinished', INITIAL);
    assert.ok(example!.tokenizeLine('send 1', partial.ruleStack).tokens.some(token => token.scopes.includes('constant.numeric.skript')));
  } finally { registry.dispose(); }
});

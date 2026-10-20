const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

function host(logger) {
  class Position {
    constructor(line, character) { this.line = line; this.character = character; }
    isEqual(other) { return this.line === other.line && this.character === other.character; }
  }
  class Range {
    constructor(start, end) { this.start = start; this.end = end; }
    contains(position) { return this.start.line === position.line && this.end.line === position.line && this.start.character <= position.character && this.end.character >= position.character; }
  }
  const position = new Position(0, 5);
  const document = { version: 1, languageId: 'skript', isClosed: false, uri: { toString: () => 'file:///test.sk' },
    getText: range => 'send '.slice(range.start.character, range.end.character) };
  const registrations = [];
  let notification, notificationDisposed = false, sdkCleared = false;
  const provider = { provideCompletionItems() {} };
  const vscode = { Position, Range, workspace: { textDocuments: [document] },
    window: { state: { focused: true }, activeTextEditor: { document, selection: { isEmpty: true, active: position } } },
    languages: { registerCompletionItemProvider: (selector, value, ...triggers) => {
      assert.equal(value, provider, 'Register the real SDK provider, never a dummy.');
      const registration = { selector, provider: value, triggers, disposed: false, dispose() { this.disposed = true; } };
      registrations.push(registration); return registration;
    } } };
  const client = { code2ProtocolConverter: { asUri: uri => uri.toString() },
    getFeature: () => ({ getProvider: () => provider, clear: () => { sdkCleared = true; } }),
    onNotification: (_method, listener) => { notification = listener; return { dispose: () => { notificationDisposed = true; } }; } };
  const module = { exports: {} };
  vm.runInNewContext(fs.readFileSync(path.resolve(__dirname, '../out/completion.js'), 'utf8'), {
    module, exports: module.exports, require: name => {
      if (name === 'vscode') return vscode;
      if (name === 'vscode-languageclient/node') return { CompletionRequest: { method: 'textDocument/completion' } };
      throw Error(`Unexpected dependency ${name}`);
    },
  });
  const bridge = new module.exports.Completion(client, [{ language: 'skript', scheme: 'file' }], () => true, document, logger);
  bridge.initialize({ completionProvider: { triggerCharacters: ['{'] } });
  const cancellationToken = () => {
    const listeners = new Set();
    return { isCancellationRequested: false,
      onCancellationRequested(listener) { listeners.add(listener); return { dispose: () => listeners.delete(listener) }; },
      cancel() { this.isCancellationRequested = true; for (const listener of [...listeners]) listener(); },
    };
  };
  const token = cancellationToken();
  const item = () => ({ label: 'send', filterText: 'send', range: new Range(new Position(0, 0), position), insertText: 'send',
    data: { completionPhase: 'prepared', completionTicket: 'opaque-ticket' } });
  const refresh = extra => notification({ uri: 'file:///test.sk', version: 1, ticket: 'opaque-ticket', position: { line: 0, character: 5 }, phase: 'final', ...extra });
  return { bridge, client, vscode, registrations, document, position, token, item, refresh,
    cancellationToken, api: module.exports, cleared: () => sdkCleared, disposed: () => notificationDisposed };
}

test('prepared completion opts in and refreshes the real provider only for the matching ticket', async () => {
  const h = host();
  const capabilities = { experimental: { other: true }, textDocument: { completion: {} } };
  h.bridge.fillClientCapabilities(capabilities);
  assert.equal(capabilities.experimental.skriptCompletionRefresh, true);
  assert.equal(capabilities.experimental.other, true);
  assert.ok(h.cleared());
  assert.ok(h.registrations[0].triggers.includes(' '));
  await h.bridge.provide(h.document, h.position, {}, h.token, async () => [h.item()]);
  h.refresh({ ticket: 'stale' }); assert.equal(h.registrations.length, 1);
  h.refresh(); assert.equal(h.registrations.length, 2);
  assert.equal(h.registrations[0].disposed, true);
  h.refresh(); assert.equal(h.registrations.length, 2, 'Consume each phase notification once.');
  h.bridge.clear(); assert.ok(h.disposed()); assert.ok(h.registrations[1].disposed);
});

test('refresh arriving during result conversion is retained and matched afterwards', async () => {
  const h = host(); let resolve;
  const pending = h.bridge.provide(h.document, h.position, {}, h.token, () => new Promise(done => { resolve = done; }));
  h.refresh(); assert.equal(h.registrations.length, 1);
  resolve([h.item()]); await pending;
  assert.equal(h.registrations.length, 2);
});

test('changed revision or cancellation never supplies an old completion result', async () => {
  for (const cancelled of [false, true]) {
    const h = host(); let resolve;
    const pending = h.bridge.provide(h.document, h.position, {}, h.token, () => new Promise(done => { resolve = done; }));
    if (cancelled) h.token.isCancellationRequested = true; else h.document.version++;
    resolve([h.item()]); assert.equal(await pending, undefined);
    h.refresh(); assert.equal(h.registrations.length, 1);
  }
});

test('different cursor, document, focus or revision cannot invalidate the active provider', async () => {
  for (const change of ['cursor', 'document', 'focus', 'version']) {
    const h = host();
    await h.bridge.provide(h.document, h.position, {}, h.token, async () => [h.item()]);
    if (change === 'cursor') h.vscode.window.activeTextEditor.selection.active = new h.vscode.Position(0, 4);
    if (change === 'document') h.vscode.window.activeTextEditor.document = {};
    if (change === 'focus') h.vscode.window.state.focused = false;
    if (change === 'version') h.document.version++;
    h.refresh(); assert.equal(h.registrations.length, 1);
  }
});

test('whitespace filter alignment preserves edit and insertion, and does not rewrite unrelated filters', () => {
  const h = host(); const item = h.item(); const range = item.range;
  h.api.alignFilter(h.document, h.position, item);
  assert.equal(item.filterText, 'send ');
  assert.equal(item.insertText, 'send'); assert.equal(item.range, range);
  const unrelated = { ...h.item(), filterText: 'other' };
  h.api.alignFilter(h.document, h.position, unrelated); assert.equal(unrelated.filterText, 'other');
});

test('a background position cannot revoke a visible prepared ticket', async () => {
  const h = host(); let calls = 0;
  await h.bridge.provide(h.document, h.position, {}, h.token, async () => [h.item()]);
  const other = new h.vscode.Position(0, 2);
  const pending = h.bridge.provide(h.document, other, {}, h.cancellationToken(), async () => { calls++; return []; });
  await Promise.resolve(); assert.equal(calls, 0);
  h.refresh(); await pending; assert.equal(calls, 1);
});

test('revocation releases the lease without claiming the next result is final', async () => {
  const h = host();
  await h.bridge.provide(h.document, h.position, {}, h.token, async () => [h.item()]);
  const other = new h.vscode.Position(0, 2);
  const pending = h.bridge.provide(h.document, other, {}, h.cancellationToken(), async () => [h.item()]);
  h.refresh({ phase: 'revoked' }); await pending;
  const count = h.registrations.length;
  h.refresh({ position: { line: 0, character: 2 } });
  assert.equal(h.registrations.length, count, 'Clean up a background ticket without reopening a different position.');
  let called = false;
  await h.bridge.provide(h.document, other, {}, h.cancellationToken(), async () => { called = true; return []; });
  assert.ok(called);
});

test('a waiting background request rechecks ownership after foreground supersession', async () => {
  const h = host(); let calls = 0;
  await h.bridge.provide(h.document, h.position, {}, h.token, async () => [h.item()]);
  const pending = h.bridge.provide(h.document, new h.vscode.Position(0, 2), {}, h.cancellationToken(), async () => { calls++; return []; });
  await h.bridge.provide(h.document, h.position, {}, h.cancellationToken(), async () => [h.item()]);
  await Promise.resolve(); assert.equal(calls, 0);
  h.refresh(); await pending; assert.equal(calls, 1);
});

test('cancel, close and clear release waiters and discard in-flight old results', async () => {
  for (const mode of ['cancel', 'close', 'clear']) {
    const h = host(); let resolve, calls = 0;
    const pending = h.bridge.provide(h.document, h.position, {}, h.token, () => new Promise(done => { resolve = done; }));
    const backgroundToken = h.cancellationToken();
    const background = h.bridge.provide(h.document, new h.vscode.Position(0, 2), {}, backgroundToken, async () => { calls++; return []; });
    if (mode === 'cancel') { backgroundToken.cancel(); h.token.cancel(); }
    if (mode === 'close') { h.document.isClosed = true; h.bridge.invalidate(h.document.uri); }
    if (mode === 'clear') h.bridge.clear();
    assert.equal(await background, undefined); assert.equal(calls, 0);
    resolve([h.item()]); assert.equal(await pending, undefined);
  }
});

test('an unrelated document does not wait for another document prepared ticket', async () => {
  const h = host();
  await h.bridge.provide(h.document, h.position, {}, h.token, async () => [h.item()]);
  const other = { ...h.document, uri: { toString: () => 'file:///other.sk' } };
  let called = false;
  await h.bridge.provide(other, h.position, {}, h.cancellationToken(), async () => { called = true; return []; });
  assert.ok(called); h.refresh(); assert.equal(h.registrations.length, 2);
});

test('document edits release a waiting background request even when no new completion is requested', async () => {
  const h = host(); let calls = 0;
  await h.bridge.provide(h.document, h.position, {}, h.token, async () => [h.item()]);
  const waiting = h.bridge.provide(h.document, new h.vscode.Position(0, 2), {}, h.cancellationToken(), async () => { calls++; return []; });
  h.document.version++; h.bridge.invalidate(h.document.uri);
  assert.equal(await waiting, undefined); assert.equal(calls, 0);
});

test('an old terminal notification cannot overwrite a matching phase during conversion', async () => {
  const h = host(); let resolve;
  const pending = h.bridge.provide(h.document, h.position, {}, h.token, () => new Promise(done => { resolve = done; }));
  h.refresh(); h.refresh({ ticket: 'older-ticket' });
  resolve([h.item()]); await pending; assert.equal(h.registrations.length, 2);
});

test('project changes revoke a matching lease even when this document version is unchanged', async () => {
  const h = host(); let calls = 0;
  await h.bridge.provide(h.document, h.position, {}, h.token, async () => [h.item()]);
  const waiting = h.bridge.provide(h.document, new h.vscode.Position(0, 2), {}, h.cancellationToken(), async () => { calls++; return []; });
  h.refresh({ phase: 'project-changed' });
  await waiting; assert.equal(calls, 1); assert.equal(h.document.version, 1);
  assert.equal(h.registrations.length, 2, 'Withdraw the obsolete prepared result and query the new project generation.');
});

test('terminal notifications during conversion never supply the superseded prepared result', async () => {
  for (const phase of ['final', 'revoked', 'project-changed', 'failed']) {
    const h = host(); let resolve;
    const pending = h.bridge.provide(h.document, h.position, {}, h.token, () => new Promise(done => { resolve = done; }));
    h.refresh({ phase }); resolve([h.item()]);
    assert.equal(await pending, undefined); assert.equal(h.registrations.length, 2);
  }
});

test('diagnostic observer failures do not alter completion results or lease cleanup', async () => {
  const h = host(() => { throw Error('Output disposed'); });
  const item = h.item();
  const result = await h.bridge.provide(h.document, h.position, {}, h.token, async () => [item]);
  assert.equal(result[0], item);
  h.refresh(); assert.equal(h.registrations.length, 2);
  h.bridge.invalidate(h.document.uri);
  h.bridge.clear();
});

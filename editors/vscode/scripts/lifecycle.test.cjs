const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const vm = require('node:vm');
const { pathToFileURL, fileURLToPath } = require('node:url');
const { createRequire } = require('node:module');

// Lifecycle-only test doubles: no server, filesystem configuration, or parser.
function host(extensionRoot) {
  const entry = path.join(extensionRoot, 'out/extension.js');
  const instances = [], errors = [], commands = new Map(), events = {};
  const uri = file => ({ scheme: 'file', fsPath: file, toString: () => pathToFileURL(file).toString() });
  const folders = ['a', 'b'].map((name, index) => ({ name, index, uri: uri(path.join(os.tmpdir(), 'client-lifecycle', name)) }));
  const document = index => ({ languageId: 'skript', uri: uri(path.join(folders[index].uri.fsPath, 'main.sk')) });
  const listen = name => callback => { events[name] = callback; return { dispose() {} }; };
  const output = { info() {}, error: value => errors.push(String(value)), show() {}, dispose() {} };
  const workspace = {
    workspaceFolders: folders, textDocuments: [document(0)], isTrusted: true,
    getWorkspaceFolder: target => folders.find(folder => path.dirname(target.fsPath) === folder.uri.fsPath),
    getConfiguration: () => ({ get: (_, fallback) => fallback }),
    onDidOpenTextDocument: listen('open'), onDidCloseTextDocument: listen('close'),
    onDidChangeWorkspaceFolders: listen('folders'), onDidChangeConfiguration: listen('config'),
    onDidGrantWorkspaceTrust: listen('trust'),
  };
  const vscode = { workspace, Uri: { file: uri, parse: value => uri(fileURLToPath(value)) },
    window: { createOutputChannel: () => output, showErrorMessage: () => Promise.resolve(undefined) },
    commands: { registerCommand: (name, callback) => { commands.set(name, callback); return { dispose() {} }; } } };
  let startupFailure = false;
  class Client {
    constructor(_id, _name, server, options) { this.server = server; this.options = options; instances.push(this); }
    registerFeature(feature) { this.feature = feature; }
    async start() { if (startupFailure) throw Error('original initialize failure'); }
    async dispose() { if (this.disposeFailure || startupFailure) throw Error('cleanup timeout'); this.disposed = true; }
  }
  const nativeRequire = createRequire(entry);
  const module = { exports: {} };
  vm.runInNewContext(fs.readFileSync(entry, 'utf8'), {
    module, exports: module.exports,
    require: name => name === 'vscode' ? vscode : name === 'vscode-languageclient/node' ? { LanguageClient: Client } : name === './config' ? { resolveConfiguration: raw => raw } : nativeRequire(name),
  }, { filename: entry });
  const context = { subscriptions: [], extension: { packageJSON: JSON.parse(fs.readFileSync(path.join(extensionRoot, 'package.json'), 'utf8')) } };
  return { api: module.exports, context, workspace, document, instances, errors, events,
    command: name => commands.get(`skriptLsp.${name}`)(), failStartup: () => { startupFailure = true; } };
}

const extensionRoot = path.resolve(__dirname, '..');
test('a failed root shutdown does not block a newly opened independent root', async () => {
  const h = host(extensionRoot);
  await h.api.activate(h.context);
  h.instances[0].disposeFailure = true;
  h.workspace.textDocuments = [h.document(1)];
  h.events.open();
  await h.command('stop'); // Drain without another reconciliation masking a failure.
  assert.equal(h.instances.length, 2);
  assert.equal(h.instances[1].options.workspaceFolder.name, 'b');
  assert.ok(h.errors.some(value => value.includes('cleanup timeout')));
  await h.api.deactivate();
});

test('cleanup failure preserves the actionable original startup error', async () => {
  const h = host(extensionRoot);
  h.failStartup();
  await h.api.activate(h.context);
  assert.ok(h.errors.some(value => value.includes('cleanup timeout')));
  assert.match(h.errors.at(-1), /original initialize failure$/);
  await h.api.deactivate();
});

test('configuration and folder changes preserve explicit Stop', async () => {
  const h = host(extensionRoot);
  await h.api.activate(h.context);
  await h.command('stop');
  h.events.config({ affectsConfiguration: () => true });
  h.events.folders();
  await h.command('stop');
  assert.equal(h.instances.length, 1);
  await h.command('start');
  assert.equal(h.instances.length, 2);
  await h.api.deactivate();
});

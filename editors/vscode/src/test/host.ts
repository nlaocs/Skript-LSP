import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';

interface Fixture { base: string; folders: Record<string, string>; loose: string; workspaceFile: string }
const checks: string[] = [];
async function eventually<T>(label: string, action: () => Thenable<T> | T, accept: (value: T) => boolean): Promise<T> {
  const deadline = Date.now() + 90000;
  let last: T | undefined;
  while (Date.now() < deadline) {
    last = await action();
    if (accept(last)) { checks.push(label); return last; }
    await new Promise(resolve => setTimeout(resolve, 250));
  }
  throw new Error(`${label} timed out; last result: ${JSON.stringify(last)}`);
}
async function open(file: string, text: string): Promise<vscode.TextDocument> {
  fs.writeFileSync(file, text);
  const document = await vscode.workspace.openTextDocument(vscode.Uri.file(file));
  await vscode.window.showTextDocument(document);
  assert.equal(document.languageId, 'skript');
  return document;
}
async function hover(document: vscode.TextDocument, line: number, column: number): Promise<string> {
  const values = await vscode.commands.executeCommand<vscode.Hover[]>('vscode.executeHoverProvider', document.uri, new vscode.Position(line, column));
  return (values ?? []).flatMap(value => value.contents.map(content => typeof content === 'string' ? content : 'value' in content ? content.value : '')).join('\n');
}
async function semantic(document: vscode.TextDocument): Promise<Uint32Array | undefined> {
  return (await vscode.commands.executeCommand<vscode.SemanticTokens>('vscode.provideDocumentSemanticTokens', document.uri))?.data;
}
export async function run(): Promise<void> {
  const configPath = process.env.SKRIPT_LSP_TEST_CONFIG;
  assert.ok(configPath, 'Use scripts/test-host.mjs with a fresh isolated profile.');
  const fixture = JSON.parse(fs.readFileSync(configPath, 'utf8')) as Fixture;
  assert.ok(vscode.workspace.workspaceFile && path.relative(vscode.workspace.workspaceFile.fsPath, fixture.workspaceFile) === '', 'Refuse a different workspace.');
  const result = path.join(fixture.base, 'result.json');
  let activated = false;
  try {
    assert.ok(vscode.workspace.isTrusted, 'The owned test workspace requires normal Workspace Trust approval.');
    const extension = vscode.extensions.getExtension('nlaocs.skript-lsp');
    assert.ok(extension);
    await extension.activate();
    activated = true;
    const doc = await open(path.join(fixture.folders.a, 'main.sk'), 'command /test:\n    permission: example\n    trigger:\n        send 1\n        invalid statement here\non join:\n    send 1\n    set {_place} to "bad"\n    teleport player to {_place}\n    send project_greet("hello") to console\n');
    await vscode.commands.executeCommand('skriptLsp.start');
    await eventually('structure executable hover', () => hover(doc, 3, 9), value => value.includes('send'));
    await eventually('structure entry hover', () => hover(doc, 1, 6), value => value.includes('Structure entry: permission'));
    await eventually('invalid statement diagnostic', () => vscode.languages.getDiagnostics(doc.uri), values => values.some(value => value.range.start.line === 4));
    await eventually('expected-type warning', () => vscode.languages.getDiagnostics(doc.uri), values => values.some(value => value.code === 'semantic.variable-type-mismatch'));
    await eventually('semantic tokens and custom legend', () => semantic(doc), value => (value?.length ?? 0) > 0);
    const legend = await vscode.commands.executeCommand<vscode.SemanticTokensLegend>('vscode.provideDocumentSemanticTokensLegend', doc.uri);
    assert.ok(legend?.tokenTypes.includes('skriptEffect')); checks.push('custom semantic roles negotiated');
    await eventually('implicit argument inlay', () => vscode.commands.executeCommand<vscode.InlayHint[]>('vscode.executeInlayHintProvider', doc.uri, new vscode.Range(0, 0, doc.lineCount, 0)), values => !!values?.some(value => JSON.stringify(value.label).includes('event player')));
    const definition = await eventually('unopened project definition', () => vscode.commands.executeCommand<(vscode.Location | vscode.LocationLink)[]>('vscode.executeDefinitionProvider', doc.uri, new vscode.Position(9, 15)), values => !!values?.length);
    assert.ok(definition.some(value => path.relative(('targetUri' in value ? value.targetUri : value.uri).fsPath, path.join(fixture.folders.a, 'library.sk')) === ''));
    await eventually('project references', () => vscode.commands.executeCommand<vscode.Location[]>('vscode.executeReferenceProvider', doc.uri, new vscode.Position(9, 15)), values => !!values?.some(value => value.uri.toString() === doc.uri.toString()));
    await eventually('signature help', () => vscode.commands.executeCommand<vscode.SignatureHelp>('vscode.executeSignatureHelpProvider', doc.uri, new vscode.Position(9, 24)), value => !!value?.signatures.length);
    const partial = await open(path.join(fixture.folders.a, 'partial.sk'), 'command /partial:\n    per');
    await eventually('validator structure completion', () => vscode.commands.executeCommand<vscode.CompletionList>('vscode.executeCompletionItemProvider', partial.uri, new vscode.Position(1, 7)), value => !!value?.items.some(item => (typeof item.label === 'string' ? item.label : item.label.label) === 'permission'));
    const second = await open(path.join(fixture.folders.b, 'main.sk'), 'on join:\n    send 2\n');
    await eventually('second root hover', () => hover(second, 1, 5), value => value.includes('send'));
    await eventually('first root retains semantic tokens', () => semantic(doc), value => !!value?.length);
    const outside = await open(path.join(fixture.loose, 'single.sk'), 'on join:\n    send 3\n');
    await eventually('loose file hover', () => hover(outside, 1, 5), value => value.includes('send'));
    const child = await open(path.join(fixture.loose, 'child', 'single.sk'), 'on join:\n    send 4\n');
    await eventually('nested loose directory hover', () => hover(child, 1, 5), value => value.includes('send'));
    await eventually('loose ancestor semantic tokens remain', () => semantic(outside), value => !!value?.length);
    await open(path.join(fixture.folders.outer, 'rejected.sk'), 'on join:\n    send 8\n');
    const nested = await open(path.join(fixture.folders['outer/nested'], 'main.sk'), 'on join:\n    send 9\n');
    await eventually('nested workspace leaf remains available', () => hover(nested, 1, 5), value => value.includes('send'));
    const cap = await open(path.join(fixture.folders.cap, 'main.sk'), 'on join:\n    send 5\n');
    await eventually('scan cap preserves open-file hover', () => hover(cap, 1, 5), value => value.includes('send'));
    await eventually('scan cap preserves semantic tokens', () => semantic(cap), value => !!value?.length);
    await vscode.commands.executeCommand('skriptLsp.stop');
    assert.equal((await hover(doc, 3, 9)), ''); checks.push('stop unregisters providers');
    await vscode.workspace.getConfiguration('skriptLsp').update('trace.server', 'messages', vscode.ConfigurationTarget.Workspace);
    await new Promise(resolve => setTimeout(resolve, 500));
    assert.equal((await hover(doc, 3, 9)), ''); checks.push('configuration change preserves explicit Stop');
    await vscode.commands.executeCommand('skriptLsp.restart');
    await eventually('restart restores providers', () => hover(doc, 3, 9), value => value.includes('send'));
    fs.writeFileSync(result, JSON.stringify({ status: 'passed', version: vscode.version, app: vscode.env.appName, checks }, null, 2));
  } catch (error) {
    fs.writeFileSync(result, JSON.stringify({ status: 'failed', version: vscode.version, checks, error: String(error) }, null, 2));
    throw error;
  } finally { if (activated) await vscode.commands.executeCommand('skriptLsp.stop'); }
}

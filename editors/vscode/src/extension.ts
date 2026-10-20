import * as path from 'node:path';
import * as vscode from 'vscode';
import { LanguageClient, Middleware, StaticFeature } from 'vscode-languageclient/node';
import { resolveConfiguration } from './config';
import { Completion } from './completion';

// The server accepts exactly one explicit file root per connection.
function rootFor(uri: vscode.Uri): vscode.WorkspaceFolder | undefined {
  if (uri.scheme !== 'file') return undefined;
  return vscode.workspace.getWorkspaceFolder(uri) ?? {
    uri: vscode.Uri.file(path.dirname(uri.fsPath)),
    name: path.basename(path.dirname(uri.fsPath)), index: 0,
  };
}

let manager: Clients | undefined;

class Clients {
  private readonly clients = new Map<string, LanguageClient>();
  private readonly failed = new Set<string>();
  private pending = Promise.resolve();
  private stopped = false;

  constructor(private readonly output: vscode.LogOutputChannel,
    private readonly tokenTypes: string[]) {}

  enqueue(action: () => Promise<void>): Promise<void> {
    this.pending = this.pending.then(action).catch(error => this.report(error));
    return this.pending;
  }

  private report(error: unknown): void {
    const message = error instanceof Error ? error.message : String(error);
    this.output.error(message);
    void vscode.window.showErrorMessage(`Skript LSP: ${message}`, 'Show Output').then(choice => {
      if (choice) this.output.show();
    });
  }

  async stop(): Promise<void> {
    this.stopped = true;
    // Dispose every owned client even if another client fails to shut down.
    const results = await Promise.allSettled([...this.clients.values()].map(client => client.dispose()));
    this.clients.clear();
    this.failed.clear();
    for (const result of results) if (result.status === 'rejected') this.report(result.reason);
  }

  async start(restart = false): Promise<void> {
    if (restart) await this.stop();
    this.stopped = false;
    this.failed.clear();
    await this.reconcile();
  }

  async reconfigure(): Promise<void> {
    const stopped = this.stopped;
    await this.stop();
    this.stopped = stopped;
    await this.reconcile();
  }

  async reconcile(): Promise<void> {
    if (this.stopped || !vscode.workspace.isTrusted) return;
    const roots = new Map<string, vscode.WorkspaceFolder>();
    for (const document of vscode.workspace.textDocuments) {
      if (document.languageId !== 'skript') continue;
      const root = rootFor(document.uri);
      if (root) roots.set(root.uri.toString(), root);
    }
    for (const [key, client] of this.clients) {
      if (roots.has(key)) continue;
      this.clients.delete(key);
      try { await client.dispose(); }
      catch (error) { this.report(error); }
    }
    for (const [key, root] of roots) {
      if (this.clients.has(key) || this.failed.has(key)) continue;
      try { await this.create(root); }
      catch (error) { this.failed.add(key); this.report(error); }
    }
  }

  private async create(root: vscode.WorkspaceFolder): Promise<void> {
    const key = root.uri.toString();
    const workspaceFolders = vscode.workspace.workspaceFolders ?? [];
    const isWorkspace = workspaceFolders.some(folder => folder.uri.toString() === key);
    if (isWorkspace && workspaceFolders.some(folder => {
      if (folder.uri.scheme !== 'file') return false;
      const relative = path.relative(root.uri.fsPath, folder.uri.fsPath);
      return relative !== '' && relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
    })) {
      throw new Error(`Nested workspace folders are not supported by this prototype. Remove either ${root.name} or its nested folder from the workspace.`);
    }
    const settings = vscode.workspace.getConfiguration('skriptLsp', root.uri);
    const config = resolveConfiguration({
      serverPath: settings.get<string>('serverPath', ''),
      snapshotPath: settings.get<string>('snapshotPath', ''),
      addonPaths: settings.get<string[]>('addonPaths', []),
    }, root.uri.fsPath);
    const owns = (uri: vscode.Uri): boolean => rootFor(uri)?.uri.toString() === key;
    const initialDocument = vscode.workspace.textDocuments.find(doc => doc.languageId === 'skript' && owns(doc.uri));
    if (!initialDocument) return;
    // Gate every document operation by the same explicit root ownership.
    let completion: Completion;
    const middleware: Middleware = {
      didOpen: async (doc, next) => { if (owns(doc.uri)) await next(doc); },
      didChange: async (event, next) => { if (owns(event.document.uri)) { completion.invalidate(event.document.uri); await next(event); } },
      didClose: async (doc, next) => { if (owns(doc.uri)) { completion.invalidate(doc.uri); await next(doc); } },
      provideCompletionItem: (doc, pos, context, token, next) => completion.provide(doc, pos, context, token, next),
      provideHover: (doc, pos, token, next) => owns(doc.uri) ? next(doc, pos, token) : undefined,
      provideSignatureHelp: (doc, pos, context, token, next) => owns(doc.uri) ? next(doc, pos, context, token) : undefined,
      provideDefinition: (doc, pos, token, next) => owns(doc.uri) ? next(doc, pos, token) : undefined,
      provideReferences: (doc, pos, context, token, next) => owns(doc.uri) ? next(doc, pos, context, token) : undefined,
      provideInlayHints: (doc, range, token, next) => owns(doc.uri) ? next(doc, range, token) : undefined,
      provideDocumentSemanticTokens: (doc, token, next) => owns(doc.uri) ? next(doc, token) : undefined,
      handleDiagnostics: (uri, diagnostics, next) => next(uri, owns(uri) ? diagnostics : []),
      workspace: {
        workspaceFolders: () => [{ uri: key, name: root.name }],
        didChangeWatchedFile: async (event, next) => { if (owns(vscode.Uri.parse(event.uri))) await next(event); },
      },
    };
    const client = new LanguageClient('skriptLsp', `Skript LSP (${root.name})`, {
      command: config.serverPath, args: ['--stdio'],
      options: { cwd: root.uri.fsPath, shell: false },
    }, {
      documentSelector: [{ scheme: 'file', language: 'skript', pattern: { baseUri: key, pattern: isWorkspace ? '**/*' : '*' } }],
      workspaceFolder: root,
      initializationOptions: { snapshotPath: config.snapshotPath, addonPaths: config.addonPaths },
      diagnosticCollectionName: `skriptLsp:${key}`,
      outputChannel: this.output, traceOutputChannel: this.output,
      middleware,
    });
    const contract: StaticFeature = {
      fillClientCapabilities: capabilities => {
        const semantic = capabilities.textDocument?.semanticTokens;
        if (semantic) semantic.tokenTypes = [...new Set([...semantic.tokenTypes, ...this.tokenTypes])];
      },
      initialize: () => {}, clear: () => {}, getState: () => ({ kind: 'static' }),
    };
    client.registerFeature(contract);
    completion = new Completion(client, [{ scheme: 'file', language: 'skript', pattern: new vscode.RelativePattern(root, isWorkspace ? '**/*' : '*') }], owns, initialDocument, (event, detail) => {
      if (vscode.workspace.getConfiguration('skriptLsp', root.uri).get('trace.server', 'off') !== 'off') {
        this.output.info(`completion ${event} ${JSON.stringify({ root: key, ...detail })}`);
      }
    });
    client.registerFeature(completion);
    try {
      await client.start();
      this.clients.set(key, client);
      this.output.info(`Started for ${key}`);
    } catch (error) {
      try { await client.dispose(); }
      catch (cleanupError) { this.report(cleanupError); }
      throw error;
    }
  }
}

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const output = vscode.window.createOutputChannel('Skript LSP', { log: true });
  context.subscriptions.push(output);
  output.info(`Activated ${context.extension.id}@${context.extension.packageJSON.version} from ${context.extensionPath} (VS Code ${vscode.version})`);
  const tokenTypes = (context.extension.packageJSON.contributes.semanticTokenTypes as { id: string }[]).map(type => type.id);
  const clients = manager = new Clients(output, tokenTypes);
  context.subscriptions.push(
    vscode.commands.registerCommand('skriptLsp.start', () => clients.enqueue(() => clients.start())),
    vscode.commands.registerCommand('skriptLsp.stop', () => clients.enqueue(() => clients.stop())),
    vscode.commands.registerCommand('skriptLsp.restart', () => clients.enqueue(() => clients.start(true))),
    vscode.commands.registerCommand('skriptLsp.showOutput', () => output.show()),
    vscode.workspace.onDidOpenTextDocument(() => { void clients.enqueue(() => clients.reconcile()); }),
    vscode.workspace.onDidCloseTextDocument(() => { void clients.enqueue(() => clients.reconcile()); }),
    vscode.workspace.onDidChangeWorkspaceFolders(() => { void clients.enqueue(() => clients.reconfigure()); }),
    vscode.workspace.onDidChangeConfiguration(event => {
      if (event.affectsConfiguration('skriptLsp')) void clients.enqueue(() => clients.reconfigure());
    }),
    vscode.workspace.onDidGrantWorkspaceTrust(() => { void clients.enqueue(() => clients.reconcile()); }),
  );
  await clients.enqueue(() => clients.reconcile());
}

export async function deactivate(): Promise<void> {
  await manager?.enqueue(() => manager!.stop());
  manager = undefined;
}

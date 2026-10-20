import * as vscode from 'vscode';
import { CompletionRequest, LanguageClient, Middleware, StaticFeature } from 'vscode-languageclient/node';

interface Refresh {
  uri: string;
  version: number;
  ticket: string;
  position: { line: number; character: number };
  phase: 'final' | 'revoked' | 'failed' | 'project-changed';
}
interface Attempt {
  document: vscode.TextDocument;
  version: number;
  position: vscode.Position;
  token: vscode.CancellationToken;
  ticket?: string;
  refreshes: Map<string, Refresh>;
  done: Promise<void>;
  finish: () => void;
  active: boolean;
  cancellation?: vscode.Disposable;
}

// Keep filtering aligned with the actual replacement prefix when the server
// preserves a right-hand suffix and omits trailing whitespace from insertion.
export function alignFilter(document: vscode.TextDocument, position: vscode.Position, item: vscode.CompletionItem): void {
  const range = item.range instanceof vscode.Range ? item.range : item.range?.inserting;
  if (!range || !range.contains(position) || item.filterText === undefined) return;
  const typed = document.getText(new vscode.Range(range.start, position));
  if (typed.length > item.filterText.length && typed.trimEnd().toLowerCase() === item.filterText.trimEnd().toLowerCase()) {
    item.filterText = typed;
  }
}

/** Owns the real SDK provider registration so a new server phase can invalidate
 * its results. Provider replacement lets Code refresh an existing suggest session
 * itself; it never executes a command that opens a dismissed suggest widget. */
export class Completion implements StaticFeature {
  private registration: vscode.Disposable | undefined;
  private notifications: vscode.Disposable | undefined;
  private provider: vscode.CompletionItemProvider | undefined;
  private triggers: string[] = [];
  private generation = 0;
  private readonly attempts = new Map<string, Attempt>();

  constructor(private readonly client: LanguageClient,
    private readonly selector: vscode.DocumentSelector,
    private readonly owns: (uri: vscode.Uri) => boolean,
    private readonly initialDocument: vscode.TextDocument) {}

  fillClientCapabilities: StaticFeature['fillClientCapabilities'] = capabilities => {
    capabilities.experimental = { ...capabilities.experimental, skriptCompletionRefresh: true };
    if (capabilities.textDocument?.completion) capabilities.textDocument.completion.dynamicRegistration = false;
  };

  initialize: StaticFeature['initialize'] = capabilities => {
    const document = this.initialDocument;
    if (!capabilities.completionProvider) throw new Error('The server did not advertise completion.');
    const feature = this.client.getFeature(CompletionRequest.method);
    this.provider = feature.getProvider(document);
    if (!this.provider) throw new Error('The language client did not register its completion provider.');
    this.triggers = [...new Set([...(capabilities.completionProvider.triggerCharacters ?? []), ' '])];
    this.replaceRegistration();
    feature.clear();
    this.notifications = this.client.onNotification('skript/completionRefresh', (value: unknown) => {
      if (!value || typeof value !== 'object') return;
      const refresh = value as Partial<Refresh>;
      if (typeof refresh.uri !== 'string' || !Number.isSafeInteger(refresh.version)
        || typeof refresh.ticket !== 'string' || !refresh.ticket
        || !refresh.position || !Number.isSafeInteger(refresh.position.line) || !Number.isSafeInteger(refresh.position.character)
        || !['final', 'revoked', 'failed', 'project-changed'].includes(refresh.phase ?? '')) return;
      const latest = this.attempts.get(refresh.uri);
      if (!latest || refresh.uri !== this.client.code2ProtocolConverter.asUri(latest.document.uri)
        || refresh.version !== latest.version || refresh.position.line !== latest.position.line
        || refresh.position.character !== latest.position.character) return;
      latest.refreshes.set(refresh.ticket, refresh as Refresh);
      this.refresh(latest);
    });
  };

  provide: NonNullable<Middleware['provideCompletionItem']> = async (document, position, context, token, next) => {
    if (!this.owns(document.uri)) return undefined;
    const version = document.version;
    const generation = this.generation;
    const uri = this.client.code2ProtocolConverter.asUri(document.uri);
    // The server owns one prepared ticket per document. Do not let a background
    // provider query at another position revoke the visible suggest session.
    for (;;) {
      if (document.isClosed || document.version !== version || token.isCancellationRequested || generation !== this.generation) return undefined;
      const previous = this.attempts.get(uri);
      if (!previous) break;
      if (previous.version === version && !previous.position.isEqual(position) && this.isCurrentEditor(previous)) {
        let cancellation: vscode.Disposable | undefined;
        await Promise.race([previous.done, new Promise<void>(resolve => {
          cancellation = token.onCancellationRequested(() => resolve());
        })]);
        cancellation?.dispose();
        // A newer foreground request may now own the lease; check again.
        continue;
      }
      this.finish(previous);
      break;
    }
    let finish!: () => void;
    const done = new Promise<void>(resolve => { finish = resolve; });
    const attempt: Attempt = { document, version, position, token, done, finish, active: true, refreshes: new Map() };
    this.attempts.set(uri, attempt);
    attempt.cancellation = token.onCancellationRequested(() => this.finish(attempt));
    try {
      const result = await next(document, position, context, token);
      if (!attempt.active || document.isClosed || document.version !== version || token.isCancellationRequested || generation !== this.generation) {
        this.finish(attempt);
        return undefined;
      }
      const items = Array.isArray(result) ? result : result?.items;
      for (const item of items ?? []) alignFilter(document, position, item);
      const tickets = new Set((items ?? []).map(item => {
        const data = (item as vscode.CompletionItem & { data?: { completionPhase?: unknown; completionTicket?: unknown } }).data;
        return data?.completionPhase === 'prepared' && typeof data.completionTicket === 'string' ? data.completionTicket : undefined;
      }).filter((ticket): ticket is string => ticket !== undefined));
      if (tickets.size === 1) {
        attempt.ticket = [...tickets][0];
        if (this.refresh(attempt)) return undefined;
      } else this.finish(attempt);
      return result;
    } catch (error) {
      this.finish(attempt);
      throw error;
    }
  };

  invalidate(uri: vscode.Uri): void {
    const attempt = this.attempts.get(this.client.code2ProtocolConverter.asUri(uri));
    if (attempt) this.finish(attempt);
  }

  private isCurrentEditor(attempt: Attempt): boolean {
    const editor = vscode.window.activeTextEditor;
    return editor?.document === attempt.document && editor.selection.isEmpty && editor.selection.active.isEqual(attempt.position);
  }

  private finish(attempt: Attempt): void {
    attempt.active = false;
    const uri = this.client.code2ProtocolConverter.asUri(attempt.document.uri);
    if (this.attempts.get(uri) === attempt) this.attempts.delete(uri);
    attempt.cancellation?.dispose();
    attempt.finish();
  }

  private refresh(attempt: Attempt): boolean {
    if (!attempt.active || !attempt.ticket || !attempt.refreshes.has(attempt.ticket)) return false;
    this.finish(attempt);
    if (attempt.document.isClosed || attempt.document.version !== attempt.version || attempt.token.isCancellationRequested
      || !vscode.window.state.focused || !this.isCurrentEditor(attempt)) return true;
    this.replaceRegistration();
    return true;
  }

  private replaceRegistration(): void {
    if (!this.provider) return;
    const previous = this.registration;
    this.registration = vscode.languages.registerCompletionItemProvider(this.selector, this.provider, ...this.triggers);
    previous?.dispose();
  }

  getState: StaticFeature['getState'] = () => ({ kind: 'static' });
  clear(): void {
    this.generation++;
    for (const attempt of this.attempts.values()) this.finish(attempt);
    this.notifications?.dispose(); this.notifications = undefined;
    this.registration?.dispose(); this.registration = undefined;
    this.provider = undefined;
  }
}

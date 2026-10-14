// Cage VSCode extension — a thin client over `cage lsp`.
//
// The server is the diagnostics-only Language Server already built into the
// cage binary (crates/cage-cli/src/lsp.rs); this extension only spawns it
// and pipes the protocol. Every diagnostic in the editor is the exact output
// of the `cage check` pipeline — same codes, locations, and hints.
//
// The two check-aligned knobs (env / level) ride the LSP initialize
// request's initializationOptions; they are read once at activation, so
// changing them needs a window reload (documented on the settings).

import * as vscode from 'vscode';
import { LanguageClient, LanguageClientOptions, ServerOptions } from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

/// Stop the server exactly once. Both the host's `deactivate` and a window
/// close (the subscription disposable) land here, so it must be idempotent and
/// must not hand back an already-stopped client.
function stopClient(): Thenable<void> | undefined {
  const current = client;
  if (!current) {
    return undefined;
  }
  client = undefined;
  return current.stop();
}

export function activate(context: vscode.ExtensionContext): void {
  const folder = vscode.workspace.workspaceFolders?.[0];
  if (!folder) {
    void vscode.window.showWarningMessage(
      'Cage: open a folder containing cage.toml to enable diagnostics.',
    );
    return;
  }

  const config = vscode.workspace.getConfiguration('cage');
  const env = config.get<string>('env') ?? '';
  const level = config.get<string>('level') ?? 'semantic';

  const serverOptions: ServerOptions = {
    command: 'cage',
    args: ['lsp', folder.uri.fsPath],
  };

  const clientOptions: LanguageClientOptions = {
    // The server pushes the project's full diagnostic panel onto every open
    // document — no language filter, any file can carry cage diagnostics.
    documentSelector: [{ scheme: 'file' }],
    initializationOptions: { env: env === '' ? undefined : env, level },
  };

  const started = new LanguageClient('cage', 'Cage', serverOptions, clientOptions);
  client = started;
  context.subscriptions.push({ dispose: () => void stopClient() });
  started.start().catch((err: unknown) => {
    client = undefined;
    void vscode.window.showErrorMessage(`Cage: failed to start cage lsp — ${String(err)}`);
  });
}

export function deactivate(): Thenable<void> | undefined {
  return stopClient();
}

// Live LSP protocol handshake: the built `cage` binary must speak exactly what
// the extension's client library speaks (initialize → didOpen →
// publishDiagnostics → shutdown → exit). This is the contract that keeps the
// editor honest — if it passes, the extension will show the cage check panel.
//
// Skipped when no prebuilt binary is found (set CAGE_BIN to override the
// default target/debug/cage).
//
// Run: npm test   (from editors/vscode/)

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, '..', '..', '..');
const badProject = join(repoRoot, 'examples', 'game-config', 'bad');
const bin = process.env.CAGE_BIN ?? join(repoRoot, 'target', 'debug', 'cage');

function frame(obj) {
  const body = JSON.stringify(obj);
  return `Content-Length: ${Buffer.byteLength(body)}\r\n\r\n${body}`;
}

/// Minimal LSP stdio session: write frames, parse reply/notification frames.
class LspSession {
  constructor(root) {
    this.proc = spawn(bin, ['lsp', root], { stdio: ['pipe', 'pipe', 'inherit'] });
    this.buffer = Buffer.alloc(0);
    this.messages = [];
    this.waiters = [];
    this.exitCode = null;
    this.proc.stdout.on('data', (chunk) => {
      this.buffer = Buffer.concat([this.buffer, chunk]);
      this.#drain();
    });
    this.proc.on('exit', (code) => {
      this.exitCode = code;
      this.#wakeAll();
    });
  }

  #drain() {
    for (;;) {
      const sep = this.buffer.indexOf('\r\n\r\n');
      if (sep === -1) return;
      const header = this.buffer.subarray(0, sep).toString('latin1');
      const m = /Content-Length:\s*(\d+)/i.exec(header);
      if (!m) throw new Error(`bad LSP header: ${header}`);
      const len = Number(m[1]);
      if (this.buffer.length < sep + 4 + len) return; // body not complete yet
      const body = this.buffer.subarray(sep + 4, sep + 4 + len).toString('utf8');
      this.buffer = this.buffer.subarray(sep + 4 + len);
      this.messages.push(JSON.parse(body));
      this.#wakeAll();
    }
  }

  #wakeAll() {
    const waiters = this.waiters.splice(0);
    for (const w of waiters) w();
  }

  send(obj) {
    this.proc.stdin.write(frame(obj));
  }

  /// Wait until `predicate` matches a message; fail the test on timeout or
  /// process exit first.
  async waitFor(predicate, what, timeoutMs = 10_000) {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      const found = this.messages.find(predicate);
      if (found) return found;
      if (this.exitCode !== null) {
        throw new Error(`cage lsp exited (${this.exitCode}) before ${what}`);
      }
      if (Date.now() >= deadline) throw new Error(`timeout waiting for ${what}`);
      await new Promise((resolve) => this.waiters.push(resolve));
    }
  }

  async stop() {
    this.send({ jsonrpc: '2.0', id: 99, method: 'shutdown' });
    await this.waitFor((m) => m.id === 99, 'shutdown reply');
    this.send({ jsonrpc: '2.0', method: 'exit' });
    const exit = await new Promise((resolve) => this.proc.on('exit', resolve));
    assert.equal(exit, 0, 'proper shutdown+exit handshake exits 0');
    this.proc.stdin.end();
  }
}

test('built cage binary speaks the protocol the extension client uses', { skip: !existsSync(bin) && `no binary at ${bin} (set CAGE_BIN)` }, async () => {
  assert.ok(existsSync(badProject), `bad example project missing: ${badProject}`);
  const session = new LspSession(badProject);
  try {
    session.send({
      jsonrpc: '2.0',
      id: 1,
      method: 'initialize',
      params: { capabilities: {}, initializationOptions: { level: 'gamerule' } },
    });
    const init = await session.waitFor((m) => m.id === 1, 'initialize reply');
    assert.equal(init.result.serverInfo.name, 'cage-lsp');
    assert.equal(init.result.capabilities.textDocumentSync.openClose, true);

    session.send({ jsonrpc: '2.0', method: 'initialized', params: {} });
    const uri = `file://${join(badProject, 'config', 'Character.csv')}`;
    session.send({
      jsonrpc: '2.0',
      method: 'textDocument/didOpen',
      params: { textDocument: { uri, languageId: 'csv', version: 1, text: '' } },
    });
    const published = await session.waitFor(
      (m) => m.method === 'textDocument/publishDiagnostics' && m.params?.uri === uri,
      'publishDiagnostics for the opened document',
    );
    const codes = published.params.diagnostics.map((d) => d.code);
    assert.ok(codes.includes('E1601'), `expected E1601 in diagnostics, got ${codes}`);
    assert.ok(
      published.params.diagnostics.some((d) => d.source === 'cage' && d.message.length > 0),
      'diagnostics carry the cage source and a message',
    );

    await session.stop();
  } finally {
    if (session.exitCode === null) session.proc.kill();
  }
});

// Cage VSCode extension contract tests (node:test, no test framework needed).
//
// Two layers, cheapest first:
//   1. package.json contract — activation, entry point, configuration keys
//   2. extension runtime — the COMPILED out/extension.js is loaded with a
//      stubbed `vscode` + `vscode-languageclient` and driven through
//      activate/deactivate, asserting on the calls it actually makes (spawn
//      command/args, initializationOptions, lifecycle). Behavioural, not
//      source-grepping, so refactors don't silently break the contract.
//
// The live protocol handshake against the built binary lives in
// handshake.test.mjs.
//
// Run: npm test   (from editors/vscode/)

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const extRoot = join(here, '..');
const repoRoot = join(extRoot, '..', '..');
const entry = join(extRoot, 'out', 'extension.js');

// --- layer 1: package.json contract ---------------------------------------

test('package.json wires activation, entry point, and settings', () => {
  const pkg = JSON.parse(readFileSync(join(extRoot, 'package.json'), 'utf8'));
  assert.equal(pkg.main, './out/extension.js', 'main entry must be the compiled JS');
  assert.ok(
    pkg.activationEvents.some((e) => e.includes('cage.toml')),
    'must activate on workspaceContains cage.toml',
  );
  assert.ok(pkg.engines?.vscode, 'engines.vscode pins the client floor');
  const props = pkg.contributes?.configuration?.properties ?? {};
  assert.ok(props['cage.env'], 'cage.env setting feeds initializationOptions.env');
  assert.ok(props['cage.level'], 'cage.level setting feeds initializationOptions.level');
  assert.equal(props['cage.level'].default, 'semantic');
});

// --- layer 2: extension runtime (compiled JS under stubbed vscode) ----------

/// Harness: records everything the extension does to its two dependencies.
function makeHarness({ folders = [{ uri: { fsPath: '/proj' } }], config = {}, startFails = false } = {}) {
  const calls = { clients: [], started: 0, stopped: 0, warnings: [], errors: [] };
  const vscode = {
    workspace: {
      workspaceFolders: folders,
      getConfiguration: () => ({ get: (key) => config[key] }),
    },
    window: {
      showWarningMessage: (m) => calls.warnings.push(m),
      showErrorMessage: (m) => calls.errors.push(m),
    },
  };
  class FakeLanguageClient {
    constructor(id, name, serverOptions, clientOptions) {
      calls.clients.push({ id, name, serverOptions, clientOptions });
    }
    start() {
      calls.started += 1;
      return startFails ? Promise.reject(new Error('spawn cage: ENOENT')) : Promise.resolve();
    }
    stop() {
      calls.stopped += 1;
      return Promise.resolve();
    }
  }
  return { calls, vscode, FakeLanguageClient };
}

/// Fresh module load per test so the extension's module-level client does not
/// leak between cases.
function loadExtension(h) {
  const require = createRequire(import.meta.url);
  const Module = require('node:module');
  const originalLoad = Module._load;
  Module._load = function (request, parent, isMain) {
    if (request === 'vscode') return h.vscode;
    if (request === 'vscode-languageclient/node') return { LanguageClient: h.FakeLanguageClient };
    return originalLoad.call(this, request, parent, isMain);
  };
  delete require.cache[require.resolve(entry)];
  try {
    return require(entry);
  } finally {
    Module._load = originalLoad;
  }
}

/// Minimal ExtensionContext: collects disposables so tests can fire them.
function makeContext() {
  const context = { subscriptions: [] };
  return context;
}

test('activate spawns `cage lsp <root>` with env/level on initializationOptions', () => {
  const h = makeHarness({ config: { env: 'prod', level: 'table' } });
  const ext = loadExtension(h);
  const context = makeContext();
  ext.activate(context);

  assert.equal(h.calls.clients.length, 1, 'exactly one language client');
  const { id, name, serverOptions, clientOptions } = h.calls.clients[0];
  assert.equal(id, 'cage');
  assert.equal(name, 'Cage');
  assert.equal(serverOptions.command, 'cage', 'spawns the cage binary by name (PATH install)');
  assert.deepEqual(serverOptions.args, ['lsp', '/proj'], 'lsp subcommand with the workspace root');
  assert.deepEqual(clientOptions.initializationOptions, { env: 'prod', level: 'table' });
  assert.deepEqual(
    clientOptions.documentSelector,
    [{ scheme: 'file' }],
    'accepts all file documents — the server pushes the full panel to each',
  );
  assert.equal(h.calls.started, 1, 'client.start() called');
  assert.deepEqual(h.calls.warnings, []);
});

test('empty cage.env means base schema (no env on the wire)', () => {
  const h = makeHarness({ config: { env: '', level: 'semantic' } });
  loadExtension(h).activate(makeContext());
  const init = h.calls.clients[0].clientOptions.initializationOptions;
  assert.equal(init.env, undefined, 'empty env must not be sent as ""');
  assert.equal('env' in init, true);
  assert.equal(init.level, 'semantic');
});

test('missing settings fall back to the check defaults (base env, semantic)', () => {
  const h = makeHarness({ config: {} });
  loadExtension(h).activate(makeContext());
  const init = h.calls.clients[0].clientOptions.initializationOptions;
  assert.equal(init.env, undefined);
  assert.equal(init.level, 'semantic');
});

test('no workspace folder warns and starts no server', () => {
  const h = makeHarness({ folders: null });
  const ext = loadExtension(h);
  ext.activate(makeContext());
  assert.equal(h.calls.clients.length, 0, 'no client without a project root');
  assert.equal(h.calls.started, 0);
  assert.equal(h.calls.warnings.length, 1, 'tells the user to open a cage project');
  assert.match(h.calls.warnings[0], /cage\.toml/);
});

test('server start failure surfaces as an error message, not a throw', async () => {
  const h = makeHarness({ startFails: true });
  loadExtension(h).activate(makeContext());
  await new Promise((r) => setImmediate(r)); // let the rejected start settle
  assert.equal(h.calls.errors.length, 1, 'failed spawn is reported');
  assert.match(h.calls.errors[0], /ENOENT/);
});

test('deactivate stops the client once and is safe to repeat', async () => {
  const h = makeHarness();
  const ext = loadExtension(h);
  ext.activate(makeContext());
  await ext.deactivate();
  assert.equal(h.calls.stopped, 1, 'deactivate stops the running client');
  assert.equal(ext.deactivate(), undefined, 'second deactivate has no client left');
});

test('window close tears the server down when deactivate never fires', async () => {
  const h = makeHarness();
  const ext = loadExtension(h);
  const context = makeContext();
  ext.activate(context);
  assert.equal(context.subscriptions.length, 1, 'dispose registered with the host');
  context.subscriptions[0].dispose();
  await new Promise((r) => setImmediate(r));
  assert.equal(h.calls.stopped, 1, 'window close stops the server');
  assert.equal(context.subscriptions[0].dispose(), undefined, 'disposing twice is a no-op');
});

test('a failed start leaves no client behind for deactivate to stop', async () => {
  const h = makeHarness({ startFails: true });
  const ext = loadExtension(h);
  ext.activate(makeContext());
  await new Promise((r) => setImmediate(r)); // let the rejected start settle
  assert.equal(ext.deactivate(), undefined, 'nothing left to stop after a failed spawn');
  assert.equal(h.calls.stopped, 0);
});

test('compiled entry point exists (npm run compile ran)', () => {
  assert.ok(existsSync(entry), `missing ${entry} — run npm run compile first`);
});

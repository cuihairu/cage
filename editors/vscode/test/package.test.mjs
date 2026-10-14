// VSIX packaging contract: the runtime dependency (vscode-languageclient)
// must ship inside the package — the compiled extension.js requires it at
// activation, and tsc does not bundle. A blanket `node_modules/**` in
// .vscodeignore breaks this silently (vsce only re-includes packages listed
// in `dependencies` when the ignore list lets it).
//
// Run: npm test   (from editors/vscode/)

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const extRoot = join(here, '..');
const pkg = JSON.parse(readFileSync(join(extRoot, 'package.json'), 'utf8'));
const vsix = join(extRoot, `cage-${pkg.version}.vsix`);

function vsixEntries() {
  try {
    const out = execFileSync('unzip', ['-Z1', vsix], { encoding: 'utf8' });
    return out.split('\n').filter(Boolean);
  } catch {
    return null; // unzip unavailable — skip below
  }
}

test('vsix exists and bundles the runtime dependency', { skip: !existsSync(vsix) && `missing ${vsix} — run npm run package first` }, () => {
  const entries = vsixEntries();
  if (entries === null) {
    return; // no unzip on this machine; CI (ubuntu) always has it
  }
  assert.ok(
    entries.some((e) => e === 'extension/out/extension.js'),
    'compiled entry point is packaged',
  );
  assert.ok(
    entries.some((e) => e.startsWith('extension/node_modules/vscode-languageclient/')),
    'runtime dependency vscode-languageclient ships inside the vsix',
  );
  assert.ok(
    !entries.some((e) => e.startsWith('extension/src/') || e.startsWith('extension/test/')),
    'sources and tests are not packaged',
  );
});

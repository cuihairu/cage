# Cage VSCode Extension

Thin client over `cage lsp` — the diagnostics-only Language Server already
built into the Cage binary. Every diagnostic you see in the editor is the
exact output of the `cage check` pipeline: same error codes, same locations,
same hints. Editor and CI never diverge.

## Requirements

- The `cage` binary on your `PATH` (install via the
  [nightly release](https://github.com/cuihairu/cage/releases/tag/nightly)
  or `scripts/install.sh`).
- VSCode 1.75+.

## Usage

Open any directory containing a `cage.toml`. The extension activates
automatically (`workspaceContains:**/cage.toml`), starts `cage lsp <project
root>` over stdio, and pushes the project's full diagnostic panel onto every
open document — the Problems view lists every error with its E code, real
location (`file:line` / `row N:col`), and fix hint.

Settings (both require a window reload — they ride the LSP `initialize`
request):

| Setting | Default | Meaning |
| --- | --- | --- |
| `cage.env` | `""` (base schema) | Environment to validate under (schema `env_overrides`) |
| `cage.level` | `semantic` | Highest validation level (`parse` … `gamerule`) |

## What it is not

No completion, hover, go-to-definition, formatting, or rename — the server
is diagnostics-only by design (those need a schema semantic model; see
`docs/design.md` §49). Diagnostics are file-level: the message body carries the
precise location, so the Problems panel is the navigation surface.

## Development

```bash
npm install
npm run compile   # tsc -p ./
npm test          # contract tests + live handshake against target/debug/cage
npx @vscode/vsce package   # produce cage-<version>.vsix
```

The VSIX rides the rolling `nightly` release as an attachment (same channel
as the binaries — never a formal version).

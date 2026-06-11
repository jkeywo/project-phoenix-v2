// Build the pure-JS client page (issue #463).
//
// The client is no longer a Bevy/WASM app — it is plain HTML + the gui/*.js
// state modules. Trunk cannot build this page without compiling the crate's
// default (server) lib into a WASM bundle and injecting an init/preload (it
// implicitly builds the local Cargo.toml even with no `rel="rust"` link), so
// we ship the client with a deterministic file copy instead.
//
// Output layout mirrors the old `client-trunk.toml` dist so the smoke suite
// (which serves dist/ and navigates to /client/#<hostId>) keeps working:
//   dist/client/index.html      (= client.html)
//   dist/client/gui/...         (JS modules + console HTML)
//   dist/client/assets/<dir>/   (runtime assets referenced by the consoles)
//   dist/client/logo.png

import { cp, mkdir, copyFile, rm } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const out = path.join(root, 'dist', 'client');

// Asset directories the client consoles load at runtime. Mirrors the
// `rel="copy-dir"` links that used to live in client.html.
const ASSET_DIRS = [
  'phone_border',
  'fonts',
  'shaders',
  'gui',
  'helm_console',
  'captain_console',
  'radar_icons',
  'sounds',
];

async function main() {
  await rm(out, { recursive: true, force: true });
  await mkdir(path.join(out, 'assets'), { recursive: true });

  // index.html ← client.html
  await copyFile(path.join(root, 'client.html'), path.join(out, 'index.html'));

  // gui/ (JS modules + console HTML + borders)
  await cp(path.join(root, 'gui'), path.join(out, 'gui'), { recursive: true });

  // assets/<dir>/
  for (const dir of ASSET_DIRS) {
    await cp(
      path.join(root, 'assets', dir),
      path.join(out, 'assets', dir),
      { recursive: true },
    );
  }

  // logo.png at the client root.
  await copyFile(path.join(root, 'assets', 'logo.png'), path.join(out, 'logo.png'));

  console.log('client page built → dist/client/ (pure JS, no WASM)');
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});

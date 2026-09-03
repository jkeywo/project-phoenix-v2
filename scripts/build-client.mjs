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
//   dist/client/assets/ship-cards/  (lobby ship-picker art — see ship-cards.mjs)
//   dist/client/logo.png

import { cp, mkdir, copyFile, rm, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { emitShipCards } from './ship-cards.mjs';
import { assertDebugSurfaceModuleCurrent } from './generate-debug-surfaces.mjs';
import { clientStampField } from './client-stamp.mjs';
import { joinCodesJson, JOIN_CODES_JSON } from './join-codes.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const out = path.join(root, 'dist', 'client');

// Asset directories the client consoles load at runtime. Mirrors the
// `rel="copy-dir"` links that used to live in client.html.
const ASSET_DIRS = [
  'strings',
  // Authored join-code format data (issue #1111) — the phone fetches it to
  // canonicalise typed suffixes and to know its own project/version GUIDs.
  'join',
  'phone_border',
  'fonts',
  'shaders',
  'gui',
  'helm_console',
  'captain_console',
  'shield_console',
  'radar_icons',
  'sounds',
];

async function main() {
  // The phone has no Rust/WASM at runtime, so its Debug Surface identity comes
  // from a committed JS module generated from the Rust catalogue. Refuse a
  // stale module before deleting the previous build output.
  await assertDebugSurfaceModuleCurrent(root);

  await rm(out, { recursive: true, force: true });
  await mkdir(path.join(out, 'assets'), { recursive: true });

  // Authored join-code data is TOML (issue #1111); the JSON its three
  // parser-free consumers read is generated from it and committed. Written
  // into assets/ rather than straight into dist/, because the Worker bundle
  // and the vitest suites read the committed copy too — the drift gate is
  // tests/client/join-codes-data.test.js.
  await writeFile(path.join(root, JOIN_CODES_JSON), await joinCodesJson(root), 'utf8');

  // index.html ← client.html, with this build's delivery stamp written into
  // the placeholder meta tag (issue #1111). The client page has no WASM to
  // bake a version into, so the same trick the `phoenix-build-demo` flag uses
  // carries protocol + content identity to it: the host reads it back over the
  // join handshake and refuses a bundle built for other content.
  const html = await readFile(path.join(root, 'client.html'), 'utf8');
  const stamp = await clientStampField(root);
  await writeFile(
    path.join(out, 'index.html'),
    html.replace(
      /(<meta\s+name="phoenix-client-stamp"\s+content=")([^"]*)(")/,
      `$1${stamp}$3`,
    ),
    'utf8',
  );

  // gui/ (JS modules + console HTML + borders)
  await cp(path.join(root, 'gui'), path.join(out, 'gui'), { recursive: true });

  // assets/<dir>/
  for (const dir of ASSET_DIRS) {
    await cp(
      path.join(root, 'assets', dir),
      path.join(out, 'assets', dir),
      {
        recursive: true,
        // assets/join/join-codes.toml is the designer-facing source (built
        // into join-codes.json just above); no client-side reader ever opens
        // the .toml, so it has no business on a phone alongside the generated
        // table it was built from.
        filter: dir === 'join' ? (src) => !src.endsWith('.toml') : undefined,
      },
    );
  }

  // Lobby ship-picker art (PRD #1023 module 4). `assets/models` is NOT in
  // ASSET_DIRS — it is ~40 MB of GLB — so the four playable hulls' captured
  // billboard atlases are resolved through their rig sidecars and copied in
  // on their own, with an index keyed by template_path. See ship-cards.mjs.
  const cards = emitShipCards(out, root);

  // logo.png / favicon.ico at the client root.
  await copyFile(path.join(root, 'assets', 'logo.png'), path.join(out, 'logo.png'));
  await copyFile(path.join(root, 'assets', 'favicon.ico'), path.join(out, 'favicon.ico'));

  console.log(
    `client page built → dist/client/ (pure JS, no WASM; ${cards} ship cards; stamp ${stamp || 'unstamped'})`,
  );
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});

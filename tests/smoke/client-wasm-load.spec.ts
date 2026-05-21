// Verify the client WASM bundle initialises cleanly:
//   - TrunkApplicationStarted fires and all wasm_client_* bindings are wired
//   - All assets referenced by the Rust client code are present in dist/client
//   - No page-level JS errors (panics, bad WASM linkage)
//
// Asset presence is checked proactively (HEAD requests from within the page),
// not reactively, so the test catches missing copy-dir entries even when
// the local WASM binary pre-dates the asset additions.

import { test, expect } from './fixtures';
import { readHostPeerId } from './fixtures';

// Sentinel asset from each directory the client Bevy app loads.
// One file per directory is enough — if the directory is missing the whole
// directory of requests will 404.  Keep this in sync with asset_server.load()
// calls in src/client/**/*.rs.
const REQUIRED_ASSETS = [
  'fonts/ChakraPetch-SemiBold.ttf',
  'fonts/JetBrainsMono-Regular.ttf',
  'phone_border/compass-ring.png',
  'phone_border/tab-corner.png',
  'gui/button-normal-idle.png',
  'gui/button-small-idle.png',
  'helm_console/panel-bg.png',
  'helm_console/joystick-pad-idle.png',
  'captain_console/panel-bg.png',
  'captain_console/red-alert-idle.png',
  'radar_icons/Icon-Ship.png',
  'radar_icons/Icon-Asteroid.png',
];

test('client WASM loads without asset 404s or JS errors', async ({ context }) => {
  // Boot the server so the client has a valid host ID to connect to.
  const serverPage = await context.newPage();
  await serverPage.goto('/');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 15_000 });
  const hostId = await readHostPeerId(serverPage);

  const clientPage = await context.newPage();
  const pageErrors: string[] = [];
  clientPage.on('pageerror', err => pageErrors.push(err.message));

  await clientPage.goto(`/client/#${hostId}`);

  // Wait for TrunkApplicationStarted to have fired and the bindings to be wired.
  await clientPage.waitForFunction(
    () => typeof (window as any).wasm_client_init === 'function',
    { timeout: 15_000 },
  );

  // Proactively verify all required asset files are reachable.
  // We use HEAD requests so Bevy doesn't need to be running yet — this catches
  // missing copy-dir entries even when the WASM binary pre-dates the additions.
  const missingAssets = await clientPage.evaluate(async (assets: string[]) => {
    const missing: string[] = [];
    await Promise.all(assets.map(async path => {
      try {
        const resp = await fetch(`/client/assets/${path}`, { method: 'HEAD' });
        if (!resp.ok) missing.push(path);
      } catch {
        missing.push(path);
      }
    }));
    return missing;
  }, REQUIRED_ASSETS);

  expect(
    missingAssets,
    'Required Bevy assets missing from dist/client/assets/.\n' +
    'Run `trunk build --release --config client-trunk.toml` to regenerate the client dist,\n' +
    'or ensure all copy-dir entries are present in client.html.',
  ).toEqual([]);

  expect(pageErrors, 'JS errors during client WASM startup').toEqual([]);
});

// Verify the client WASM bundle initialises cleanly:
//   - TrunkApplicationStarted fires and all wasm_client_* bindings are wired
//   - All assets referenced by the Rust client code are present in dist/client
//   - No page-level JS errors (panics, bad WASM linkage)
//   - No Bevy hierarchy warnings (B0004) in the browser console
//
// Asset presence is checked proactively (HEAD requests from within the page),
// not reactively, so the test catches missing copy-dir entries even when
// the local WASM binary pre-dates the asset additions.

import { test, expect, readHostPeerId, waitForWasmReady } from './fixtures';

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
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);

  const clientPage = await context.newPage();
  const pageErrors: string[] = [];
  const bevyWarnings: string[] = [];
  // console.error messages from the browser page (e.g. console_error_panic_hook output).
  const browserConsoleErrors: string[] = [];
  clientPage.on('pageerror', err => pageErrors.push(err.message));
  clientPage.on('console', msg => {
    if (msg.type() === 'error') {
      browserConsoleErrors.push(msg.text());
    }
    // Capture Bevy log output: warn!() maps to console.warn in WASM builds.
    if (msg.type() === 'warning') {
      const text = msg.text();
      // B0004 = Bevy hierarchy inconsistency (ChildOf without matching Children).
      if (text.includes('B0004')) bevyWarnings.push(text);
    }
  });

  await clientPage.goto(`/client/#${hostId}`);

  // Wait for TrunkApplicationStarted to have fired and the bindings to be wired.
  await clientPage.waitForFunction(
    () => typeof (window as any).wasm_client_init === 'function',
    { timeout: 15_000 },
  );

  // Give Bevy a few frames to run its startup systems so hierarchy warnings
  // have a chance to fire before we assert.
  await clientPage.waitForTimeout(2_000);

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

  const panicContext = browserConsoleErrors.length > 0
    ? '\nBrowser console.error output (panic hook / WASM errors):\n' + browserConsoleErrors.join('\n')
    : '';
  expect(pageErrors, 'JS errors during client WASM startup' + panicContext).toEqual([]);

  expect(
    bevyWarnings,
    'Bevy B0004 hierarchy warnings during client startup.\n' +
    'An entity has a ChildOf component whose parent lacks Children — fix despawn/reparent logic.',
  ).toEqual([]);
});

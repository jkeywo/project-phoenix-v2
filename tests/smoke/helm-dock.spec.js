// Issue #1159, extended by issue #1388 — Smoke test: the contextual helm dock
// control, on the two sides of the contract it exists to keep.
//
// This is the browser-level smoke the ACs ask for. It runs under the same
// Playwright harness as the other `tests/smoke/*.spec.js` (WASM host + real
// shimmed-transport clients), so it is GATE/CI work, not part of the cheap
// `npx vitest run` unit pass — building the WASM host and launching browsers is
// expensive and is not run inline during development.
//
// WHAT IT COVERS
//
//   The dock control is CONTEXTUAL, in two directions, and each hull proves one:
//
//   * ABSENT — a hull whose Helm owns no `kind = "dock"` System is never sent a
//     `dock` blackboard at all, so its console renders no dock control and
//     nothing about the helm console breaks for a hull without a dock. The
//     Alliance BATTLESHIP is that hull: it authors no `[dock]` table and no dock
//     system, which is why this half now serves its own fixture world rather
//     than the suite's default one. Before #1388 the default world's own cruiser
//     WAS the dockless hull; the cruiser docks now, so a spec still leaning on
//     the default world would be asserting the opposite of what it claims.
//
//   * PRESENT — the CRUISER (#1388) authors both, so with a berth drawn up
//     alongside, its real Helm console shows the Dock control and pressing it
//     mates the two hulls. This half drives the REAL client page and the real
//     `gui/cruiser/helm.html` iframe, so what it asserts is the console's own
//     contract: `#dock-panel` appears, `#dock-btn` reads Dock, and after the
//     press that same button carries the `docked` class and reads Undock —
//     which it can only do once the server has published a mated dock.
//
// NEITHER HALF IS @core. The dock FEATURE's breadth entry is
// `operations-dock-umbilical.spec.js` (Helm dock -> Engineering umbilical ->
// capacity moves); these two are per-hull variants of a covered feature, which
// AGENTS.md's two-tier rule puts in the nightly/full tier rather than the PR
// breadth pass.
//
// The complementary non-browser proofs, unchanged:
//   * The headless probe `two_hulls_reach_a_mated_dock_...` in
//     `tests/headless_runner.rs` proves the SERVER flips `available_target` as
//     the berth enters and leaves the authored range.
//   * The `helm dock control (issue #1159)` vitest block in
//     `tests/client/console-state.test.js` proves the CLIENT's
//     `buildHelmConsoleState(...).dock` view appears when `available` and
//     becomes the undock control when `docked`, and `helm-console.test.js`
//     drives both dock-capable hulls' `renderStation` over that view.

import {
  test,
  expect,
  readHostPeerId,
  createTestClient,
  waitForWasmReady,
  expectFixtureWorld,
} from './fixtures';
import { ts } from './strings';

// A hull that authors no dock, as the player ship. The Alliance battleship is
// the shipped hull with no `[dock]` table and no `kind = "dock"` system, so it
// is the one that can still prove the ABSENT half. The single
// `[[available_ships]]` entry takes server.html's 'auto-select' branch; without
// it the host takes the 'legacy-fallback' branch and station-gate-checks a
// hardcoded `alliance_cruiser.toml` whose include closure this world never
// preloads, faulting boot before __wasmReady (see
// operations-dock-umbilical.spec.js's DOCK_WORLD for the same note).
const NO_DOCK_WORLD = `
[global]
seed = 1388
title = "Helm Dock Absent Fixture"

[ambient_light]
color      = [0.6, 0.55, 0.5]
brightness = 300.0

[[available_ships]]
template_path = "assets/entities/alliance_battleship.toml"

[[entity]]
template_path = "assets/entities/alliance_battleship.toml"
id = "player-ship"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"
`;

// The cruiser and one passive berth drawn up 70 units to starboard — well inside
// the cruiser's authored 200-unit dock range, so the control is in range from
// the first tick and the mate manoeuvre is short.
//
// The berth is spawned at game_start (like the player ship) rather than at world
// load: its dock markers come from the `dock_probe` model-variant rig sidecar,
// and only a game_start spawn is gated behind the browser asset preload that
// delivers that sidecar first. A world-load spawn races the async sidecar fetch
// and can come up with no DockMarkers (there is no re-resolve), leaving the
// berth permanently un-dockable in the browser host. The native config cache
// reads the sidecar synchronously, so this only bites the WASM smoke.
const CRUISER_DOCK_WORLD = `
[global]
seed = 1388
title = "Cruiser Helm Dock Fixture"

[ambient_light]
color      = [0.6, 0.55, 0.5]
brightness = 300.0

[[available_ships]]
template_path = "assets/entities/alliance_cruiser.toml"

[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id = "player-ship"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"

[[entity]]
template_path = "assets/entities/dock_berth.toml"
name = "world.probe_dock.entity.berth.name"
transform = { position = [70.0, 0.0, 0.0] }
spawn_on = "game_start"
`;

async function waitForStation(client, timeout = 5_000) {
  await client.page.waitForFunction(
    (t) =>
      window.__messages?.some(
        (m) => m.type === 'StationAssigned' && m.data.token === t,
      ),
    client.token,
    { timeout },
  );
}

test('helm receives no dock blackboard on a hull with no dock system', async ({
  context,
}) => {
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: NO_DOCK_WORLD }),
  );

  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);

  const helm = await createTestClient(context, hostId, { name: 'Helm' });
  await helm.send('SelectStation', { station: 'Helm' });
  await waitForStation(helm);
  await helm.send('SetReady', { ready: true });
  await helm.waitForMessage('GameStarted', 10_000);

  expectFixtureWorld(await helm.waitForMessage('WorldSetup', 5_000), NO_DOCK_WORLD);

  // Let a few ticks of blackboards flow, then assert none carried a `dock`
  // entry — a hull with no dock system publishes no dock blackboard, so the
  // contextual control is absent.
  await helm.page.waitForTimeout(1_000);
  const sawDock = await helm.page.evaluate(() =>
    (window.__messages || []).some(
      (m) =>
        m.type === 'BlackboardUpdate' &&
        (m.data.updates || []).some(([systemId]) => systemId === 'dock'),
    ),
  );
  expect(sawDock).toBe(false);

  await helm.close();
});

test('cruiser helm shows the Dock control in range and docks with a berth', async ({
  context,
}) => {
  test.setTimeout(120_000);

  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: CRUISER_DOCK_WORLD }),
  );

  const serverPage = await context.newPage();
  const serverCrashes = [];
  serverPage.on('crash', () => serverCrashes.push('server page crashed'));
  serverPage.on('pageerror', (err) => serverCrashes.push(err.message));

  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);

  // A second, lightweight client on the same host reads the wire, so the world
  // this spec actually served can be asserted the way fixtures.js asks every
  // routed fixture to be. It takes no Station — a stationless participant still
  // counts for collective readiness (`lobby::session::readiness_tally`), which
  // is also why it MUST ready up: an unready connected crew row blocks the
  // auto-start and the real client below would sit in the lobby for ever.
  const wire = await createTestClient(context, hostId, { name: 'Wire' });

  // The real client page, claiming Helm the way a player does.
  const helm = await context.newPage();
  const helmErrors = [];
  helm.on('pageerror', error => helmErrors.push(error.message));
  await helm.goto(`/client/#${hostId}`, { waitUntil: 'domcontentloaded' });
  await helm.waitForSelector('#station-list .station-row', { timeout: 15_000 });
  await helm.click('#station-list .station-row:has-text("Helm") button.claim-btn');
  await helm.waitForSelector('#ready-btn:not([style*="display: none"])', { timeout: 5_000 });
  await helm.click('#ready-btn');
  await wire.send('SetReady', { ready: true });
  await expect(helm.locator('#helm-ui')).toHaveClass(/active/, { timeout: 20_000 });

  expectFixtureWorld(await wire.waitForMessage('WorldSetup', 15_000), CRUISER_DOCK_WORLD);

  // ── The control appears, because a berth is in range ───────────────────────
  const helmFrame = helm.frameLocator('#helm-iframe');
  const dockPanel = helmFrame.locator('#dock-panel');
  const dockBtn = helmFrame.locator('#dock-btn');
  await expect(dockPanel).toBeVisible({ timeout: 45_000 });
  await expect(dockBtn).toHaveText(ts('console.dock.dock'));
  await expect(helmFrame.locator('#dock-status'))
    .toContainText(ts('console.dock.available'));

  // ── Pressing it mates the two hulls ────────────────────────────────────────
  // Re-pressed until the mate forms: the manoeuvre takes several ticks and a
  // throttled headless server may drop the first press. A repeat while the
  // approach is already flying is refused server-side and changes nothing, and
  // the guard below means the button is never pressed once it says Undock.
  for (let attempt = 0; attempt < 20; attempt += 1) {
    const cls = (await dockBtn.getAttribute('class')) || '';
    if (cls.includes('docked')) break;
    await dockBtn.click();
    expect(helmErrors, 'Dock click must not throw in the console').toEqual([]);
    await helm.waitForTimeout(1_500);
  }
  await expect(dockBtn, 'the two hulls must reach a mated dock')
    .toHaveClass(/docked/, { timeout: 20_000 });
  await expect(dockBtn).toHaveText(ts('console.dock.undock'));
  await expect(helmFrame.locator('#dock-status'))
    .toContainText(ts('console.dock.docked'));

  expect(serverCrashes, `server errors: ${serverCrashes.join('; ')}`).toEqual([]);

  await wire.close();
});

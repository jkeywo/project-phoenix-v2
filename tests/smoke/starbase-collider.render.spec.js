// Verification for the starbase-collider-oversize fix.
//
// combat_test's Starbase Alpha uses `station_axiom.toml`, whose `[collider]` is
// an authored Cylinder radius 17.04 (its true visible half-extent). But under
// the real render path, `update_mesh_lod` stamps the model's `[base].scale`
// ([15, 18, 18]) onto the STATION ENTITY's own `Transform` at every non-near LOD
// tier, and rapier's `apply_scale` folds `GlobalTransform.scale` into the
// collider shape — inflating the 17.04 disc to a ~290-unit one. Headless never
// runs an LOD system, so it never saw this and no digest recorded it; only the
// browser did, which is where John hit it.
//
// The player ship spawns at [400, 0, 200] and the starbase at [500, 0, 100] —
// 141 units apart. That is well outside the true 17-unit hull but DEEP INSIDE
// the ~290-unit inflation, so pre-fix the ship spawned inside the collider: it
// was de-overlapped and took collision `DamageTaken` at game start, in clear
// sky 141 units from a station 34 units across. Post-fix (colliders pinned to
// `ColliderScale::Absolute(ONE)`), 141 > 17.04, so the ship spawns free and
// takes no damage.
//
// # Why the `render` project
//
// The bug lives in the render/LOD path, which `src/server/bridge.rs` skips under
// `navigator.webdriver`. This spec hides that flag so the real path runs (the
// `render` project supplies a SwiftShader GL context). The message-suite
// `chromium` project ignores `*.render.spec.js`, so this only runs where the LOD
// system is actually alive.

import {
  test,
  expect,
  readHostPeerId,
  createTestClient,
  waitForWasmReady,
} from './fixtures';

// Starbase Alpha's authored position (assets/worlds/combat_test.toml) and its
// true visible half-extent (station_axiom.toml `[collider] radius`).
const STARBASE = { x: 500, y: 0, z: 100 };
const VISIBLE_RADIUS = 17.04;
// The player ship's spawn is ~141 u from the starbase. Pre-fix inflation was
// ~290 u, so "did the collider swallow the spawn" splits cleanly at, say, 60 u:
// post-fix the ship sits near 141, pre-fix it was dragged to the ~18 u surface.
const SPAWN_DISTANCE = Math.hypot(500 - 400, 100 - 200); // ≈ 141.42

/** Distance from an entity_states physics row to the starbase centre. */
function distToStarbase(physics) {
  const [x, , z] = physics;
  return Math.hypot(x - STARBASE.x, z - STARBASE.z);
}

/** The player ship's live distance to the starbase, read from the newest helm
 *  BlackboardUpdate on the client (issue #570 moved ship position there; the
 *  `helm` blackboard carries the LocalShip's `x`/`z`). */
async function shipDistanceToStarbase(client) {
  return client.page.evaluate(() => {
    const msgs = window.__messages || [];
    for (let i = msgs.length - 1; i >= 0; i -= 1) {
      const m = msgs[i];
      if (m.type !== 'BlackboardUpdate') continue;
      const helm = (m.data.updates || []).find(([id]) => id === 'helm');
      if (!helm) continue;
      const bb = helm[1]?.data;
      if (bb && typeof bb.x === 'number' && typeof bb.z === 'number') {
        return Math.hypot(bb.x - 500, bb.z - 100);
      }
    }
    return null;
  });
}

test.describe('starbase collider matches its visible size', () => {
  test.describe.configure({ timeout: 300_000 });

  test('the destroyer spawns clear of the starbase and takes no ram damage', async ({
    context,
  }) => {
    const serverPage = await context.newPage();

    // Take the real render/LOD path (see header).
    await serverPage.addInitScript(() => {
      Object.defineProperty(navigator, 'webdriver', { get: () => false });
    });

    await serverPage.goto('/?scenario=assets/worlds/combat_test.toml');

    // Pick the Alliance Destroyer card (combat_test offers four hulls).
    const destroyer = serverPage
      .locator('#scenario-panel ph-ship-picker .ship-card', { hasText: 'Destroyer' })
      .first();
    await destroyer.waitFor({ state: 'visible', timeout: 30_000 });
    await destroyer.click();

    await waitForWasmReady(serverPage, 120_000);

    const gl = await serverPage.evaluate(
      () => !!document.createElement('canvas').getContext('webgl2'),
    );
    expect(gl, 'SwiftShader supplied a WebGL2 context').toBe(true);

    const hostId = await readHostPeerId(serverPage);
    const helm = await createTestClient(context, hostId, { name: 'Helm' });

    await helm.send('SelectStation', { station: 'Helm' });
    await helm.page.waitForFunction(
      (t) => window.__messages?.some((m) => m.type === 'StationAssigned' && m.data.token === t),
      helm.token,
      { timeout: 30_000 },
    );
    await helm.send('SetReady', { ready: true });
    await helm.waitForMessage('GameStarted', 60_000);

    await serverPage.bringToFront();

    // Let the sim run a few seconds of real time: enough for the station to
    // resolve its LOD tier (and, pre-fix, to inflate its collider and drag the
    // spawn in) and for any collision `DamageTaken` to have fired.
    await helm.waitForMessage('BlackboardUpdate', 10_000);
    await serverPage.waitForTimeout(5_000);

    const dist = await shipDistanceToStarbase(helm);
    const damagePreview = await helm.page.evaluate(
      () => (window.__messages || []).filter((m) => m.type === 'DamageTaken').length,
    );
    // eslint-disable-next-line no-console
    console.log(
      `STARBASE-COLLIDER: ship ${dist == null ? 'null' : dist.toFixed(1)} u from starbase, ` +
        `${damagePreview} DamageTaken event(s) in first ~5 s`,
    );
    expect(dist, 'a helm BlackboardUpdate with the ship position must have arrived').not.toBeNull();

    // The ship must still be out near its spawn, NOT dragged onto the hull by an
    // inflated collider. A generous floor (60 u) clears spawn drift while still
    // failing hard on the pre-fix ~18 u surface pin.
    expect(
      dist,
      `the destroyer must stay clear of the starbase (spawned ${SPAWN_DISTANCE.toFixed(
        0,
      )} u out; visible hull radius ${VISIBLE_RADIUS}); pre-fix it was dragged to ~18 u`,
    ).toBeGreaterThan(60);

    // The decisive signal: no collision damage from simply existing 141 u from
    // the station. `DamageTaken` is the player-only collision/hit message; the
    // allied station never fires on the player and wave 1 spawns >1000 u away,
    // so any early `DamageTaken` here is the collider swallowing the spawn.
    const damageEvents = await helm.page.evaluate(
      () => (window.__messages || []).filter((m) => m.type === 'DamageTaken').length,
    );
    expect(
      damageEvents,
      'the destroyer must take NO ram damage sitting 141 u from a 34-u-wide station',
    ).toBe(0);

    await serverPage.screenshot({
      path: 'target/starbase-collider-after.png',
    });

    await helm.close();
  });
});

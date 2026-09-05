// Issue #1396 — a COLD weapons group cannot fire.
//
// Restraint moved from the captain's hidden Weapons Hold toggle to the reactor
// (PRD #1371, plan §8.3): a power group authored with `min_level = 0` may be
// commanded to level 0, and while it is there the systems that draw from it do
// not shoot. This spec drives that end to end through the real seams — an
// Engineering client's `SetPowerGroupAllocation`, a Tactical client's
// `FirePhaser` — rather than through a Rust fixture, because the claim is about
// what the crew can and cannot do to each other's stations.
//
// Both halves are asserted in ONE run on ONE hull, with nothing changing between
// them but the reactor level:
//   1. weapons at 0 → the fire order produces no `BeamStarted` at all;
//   2. weapons back at its boot level → the identical order lights the beam.
// The second half is what makes the first half's negative assertion mean
// something: a fixture that could never fire would pass step 1 for the wrong
// reason.

import {
  test,
  expect,
  readHostPeerId,
  createTestClient,
  waitForWasmReady,
  expectFixtureWorld,
} from './fixtures';

// Self-contained smoke-test world, the same shape `tactical-fire-flow.spec.js`
// uses: the player's cruiser at the origin facing -Z, one hostile 15.8 units off
// the port bow — inside the fore bank's arc and well inside its reach — so
// nothing has to be steered. The cruiser is the hull that authors
// `[power_groups.weapons] min_level = 0`, which is what makes it coldable.
const MINIMAL_TEST_WORLD = `
[global]
seed = 42
title = "Weapons Cold Fixture"

[ambient_light]
color      = [0.6, 0.55, 0.5]
brightness = 300.0

[anchors]
patrol_alpha = [600.0, 0.0, -600.0]
patrol_beta  = [500.0, 0.0, -300.0]
patrol_gamma = [200.0, 0.0, -600.0]

[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id = "player-ship"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"
overrides = { tags = ["ship"] }

[[entity]]
template_path = "assets/entities/ship_harrow_patrol.toml"
name          = "raider_alpha"
transform     = { position = [-15.0, 0.0, -5.0] }
spawn_on      = "game_start"
`;

const POWER_GROUP = 'weapons';

/** Send SelectStation and wait for a StationAssigned for *this* client's token. */
async function selectAndWait(client, station, timeout = 5_000) {
  await client.send('SelectStation', { station });
  await client.page.waitForFunction(
    (t) => window.__messages?.some(
      (m) => m.type === 'StationAssigned' && m.data.token === t,
    ),
    client.token,
    { timeout },
  );
}

/** Ask Engineering's reactor for an absolute level on the weapons group. */
async function setWeaponsPower(engineering, level) {
  await engineering.send('ControlSystem', {
    target: 'power-reactor',
    payload: {
      type: 'SetPowerGroupAllocation',
      data: { group: POWER_GROUP, level },
    },
  });
}

/** Wait until the newest PowerState reports the weapons group at `level`. */
async function waitForWeaponsPower(engineering, level, timeout = 15_000) {
  await engineering.page.waitForFunction(
    (want) => {
      const msgs = window.__messages || [];
      const last = msgs.filter((m) => m.type === 'PowerState').pop();
      return !!last && last.data.weapons === want;
    },
    level,
    { timeout },
  );
}

/** How many `BeamStarted` messages this client has seen. */
function beamCount(client) {
  return client.page.evaluate(
    () => (window.__messages || []).filter((m) => m.type === 'BeamStarted').length,
  );
}

async function startGame(context) {
  // Serve the self-contained world instead of the real default.toml.
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: MINIMAL_TEST_WORLD }),
  );

  // Make the raider inert: idle, stationary, and with its enemy_in_range
  // transition disabled, so the ONLY thing in this world that can produce a
  // `BeamStarted` is the player's own fire order. The negative assertion counts
  // beams, so a hostile taking pot shots would fail it for the wrong reason.
  await context.route('**/assets/entities/ship_harrow_patrol.toml', async (route) => {
    const response = await route.fetch();
    const text = await response.text();
    const patched = text
      .replace(/initial_state\s*=\s*"patrol"/, 'initial_state = "idle"')
      .replace(/target_speed\s*=\s*[\d.]+/g, 'target_speed = 0.0')
      .replace(/condition = "enemy_in_range"/, 'condition = "never_matches"');
    await route.fulfill({ contentType: 'text/plain', body: patched });
  });

  const serverPage = await context.newPage();
  const serverCrashes = [];
  serverPage.on('crash', () => { serverCrashes.push('server page crashed'); });
  serverPage.on('pageerror', (err) => { serverCrashes.push(err.message); });

  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);

  // Three stations, three claims: Helm to fly, Tactical to shoot, Engineering to
  // hold the reactor. Engineering must be HUMAN-held — an AI-backfilled Power
  // officer would restore the group its authored policy wants and warm the guns
  // back up mid-test.
  const helm = await createTestClient(context, hostId, { name: 'Helm' });
  const tactical = await createTestClient(context, hostId, { name: 'Tac' });
  const engineering = await createTestClient(context, hostId, { name: 'Eng' });

  await selectAndWait(helm, 'Helm');
  await selectAndWait(tactical, 'Tactical');
  await selectAndWait(engineering, 'Engineering');

  await helm.send('SetReady', { ready: true });
  await tactical.send('SetReady', { ready: true });
  await engineering.send('SetReady', { ready: true });
  await tactical.waitForMessage('GameStarted', 10_000);
  await engineering.waitForMessage('GameStarted', 10_000);

  return { helm, tactical, engineering, serverCrashes };
}

test('a cold weapons group cannot fire, and power gives the fire back', { tag: '@core' }, async ({ context }) => {
  test.setTimeout(180_000);
  const { helm, tactical, engineering, serverCrashes } = await startGame(context);

  // The raider's uuid, off WorldSetup.
  const worldSetup = await tactical.waitForMessage('WorldSetup', 5_000);
  expectFixtureWorld(worldSetup, MINIMAL_TEST_WORLD);
  const entities = worldSetup?.data?.world?.entities ?? [];
  const raider = entities.find(
    (e) => Array.isArray(e.tags) && e.tags.includes('npc'),
  );
  expect(raider, 'raider entity must appear in WorldSetup').toBeDefined();
  const raiderUuid = raider.uuid;

  // The level the hull BOOTS its weapons group at, read off the wire rather than
  // pinned to the TOML (issue #941): the test restores what it found.
  await engineering.page.waitForFunction(
    () => (window.__messages || []).some(
      (m) => m.type === 'PowerState' && typeof m.data.weapons === 'number',
    ),
    undefined,
    { timeout: 15_000 },
  );
  const bootLevel = await engineering.page.evaluate(
    () => (window.__messages || []).filter((m) => m.type === 'PowerState').pop().data.weapons,
  );
  expect(bootLevel, 'the cruiser must boot its weapons group above cold').toBeGreaterThan(0);

  // Lock the raider and wait until a bank reports itself fire-ready, so every
  // gate but the reactor is known to be open before the guns are switched off.
  await tactical.send('ControlSystem', {
    target: 'tactical-radar',
    payload: { type: 'SetTarget', data: { uuid: raiderUuid } },
  });
  await tactical.page.bringToFront();
  await tactical.page.waitForFunction(
    () => window.__messages?.some(
      (m) => m.type === 'WeaponsUpdate'
        && Array.isArray(m.data.banks)
        && m.data.banks.some((b) => b.fire_ready === true),
    ),
    undefined,
    { timeout: 20_000 },
  );

  // ── COLD ────────────────────────────────────────────────────────────────
  await setWeaponsPower(engineering, 0);
  await waitForWeaponsPower(engineering, 0);
  expect(await beamCount(tactical), 'nothing has fired before the order').toBe(0);

  await tactical.send('ControlSystem', {
    target: 'phaser-fore',
    payload: { type: 'FirePhaser' },
  });
  // A generous window: the host page is backgrounded, so its sim runs well
  // behind wall clock. The powered half below fires inside a shorter one, which
  // is what says this window was long enough to have seen a beam.
  await tactical.page.waitForTimeout(10_000);
  expect(
    await beamCount(tactical),
    'weapons at level 0: the fire order must produce no beam at all',
  ).toBe(0);

  // ── POWERED ─────────────────────────────────────────────────────────────
  await setWeaponsPower(engineering, bootLevel);
  await waitForWeaponsPower(engineering, bootLevel);

  await tactical.send('ControlSystem', {
    target: 'phaser-fore',
    payload: { type: 'FirePhaser' },
  });
  const beamStarted = await tactical.waitForMessage('BeamStarted', 20_000);
  expect(beamStarted.data.target_uuid).toBe(raiderUuid);

  expect(serverCrashes).toEqual([]);

  await helm.close();
  await tactical.close();
  await engineering.close();
});

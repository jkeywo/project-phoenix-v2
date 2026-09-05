// Issue #1397 — Smoke test: one ship's weapons are COLD, another ship's Sensors
// say so.
//
// Restraint is expressed as POWER (issue #1395): a hull may author its weapons
// group with a floor of 0 and boot it switched off, and a ship that has done
// that cannot fire. This spec proves the OTHER half of that — that the fact is
// legible from outside the hull, on the one surface allowed to see it.
//
// The chain, end to end on the real WASM host over the actual wire:
//
//   1. SHIP A ("cold_hauler") boots with its `weapons` power group at level 0.
//      No AI arm can quietly raise it: `plan_allocation` filters cold groups out
//      of the bidding entirely, so what the hull authored is what stays true.
//   2. SHIP B is the player cruiser. Its SCIENCE seat — the seat that owns the
//      `sensors` and `sensor-radar` systems on that hull — designates ship A,
//      sending exactly the ControlSystem envelope `set_sensors_target` sends
//      (gui/action-map.js).
//   3. SHIP B's sensor-radar blackboard comes back carrying
//      `selected_target_weapons_cold: true` — the target-scoped replica the host
//      publishes only for a selection whose reactor actually tracks a weapons
//      group.
//   4. THE OPERATOR READS IT: that live blackboard goes through the real client
//      builder (`buildSensorsConsoleState`) into the real shipped Sensors
//      console, which paints a WEAPONS / COLD row on the scan card.
//
// Then the same four steps against a second, otherwise identical hull whose
// weapons are up, which must read POWERED. Two targets rather than one because
// a constant would pass the first assertion on its own: what is under test is a
// per-target lookup keyed on the selection, not a field that happens to be set.

import {
  test,
  expect,
  readHostPeerId,
  createTestClient,
  waitForWasmReady,
  expectFixtureWorld,
} from './fixtures';
import { ts } from './strings';

// A self-contained world: the player cruiser plus two Harrow tugs.
//
// `ship_harrow_tug.toml` is the NPC hull here because it is the one shipped
// NPC that authors a real `[power_groups.*]` block including `weapons`, so the
// cold ship needs a two-key override rather than a whole reactor written out in
// the fixture. Its doctrine is `hold-station`, so neither tug goes looking for a
// fight and nothing in the run can change the levels under the assertions.
//
// `min_level = 0` alongside `default_level = 0` is not belt-and-braces:
// `PowerSystem::from_authored_groups` clamps the boot level to the group's own
// floor, so a level of 0 under the hull's default floor of 1 would seed at 1 —
// warm, and the spec would be asserting nothing.
const WEAPONS_COLD_WORLD = `
[global]
seed = 1397
title = "Sensors Weapons-Cold Fixture"
description = "Player cruiser + one cold hull + one armed hull; see tests/smoke/sensors-weapons-cold.spec.js."

[ambient_light]
color      = [0.6, 0.55, 0.5]
brightness = 300.0

[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id            = "player-ship"
transform     = { position = [0.0, 0.0, 0.0] }
spawn_on      = "game_start"
overrides     = { tags = ["ship"] }

# SHIP A — weapons switched off at the reactor.
[[entity]]
template_path = "assets/entities/ship_harrow_tug.toml"
id            = "cold-hauler"
name          = "cold_hauler"
transform     = { position = [300.0, 0.0, -300.0] }
spawn_on      = "game_start"
overrides     = { tags = ["ship", "npc"], power_groups = { weapons = { default_level = 0, min_level = 0 } } }

# The control: the same hull with its weapons group left as the template
# authors it (level 2). Proves the reading is looked up per selection.
[[entity]]
template_path = "assets/entities/ship_harrow_tug.toml"
id            = "armed-hauler"
name          = "armed_hauler"
transform     = { position = [-300.0, 0.0, -300.0] }
spawn_on      = "game_start"
overrides     = { tags = ["ship", "npc"] }
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

// The uuid the host assigned the world entity authored with `id`.
//
// Read off WorldSetup rather than guessed: uuids come from the spawner, and the
// per-tick SimState snapshot carries no authored identity to match on. Matching
// on the authored `id` rather than on position in the list keeps the spec honest
// if the fixture ever grows another hull.
async function uuidOfWorldEntity(client, id) {
  const setup = await client.lastMessage('WorldSetup');
  const hit = ((setup?.data?.world?.entities) || []).find((e) => e.id === id);
  return hit?.uuid || null;
}

// Designate `uuid` as the Science Target and wait for this ship's own
// sensor-radar blackboard to come back describing it. Returns that blackboard.
async function designateAndReadRadar(client, uuid) {
  let bb = null;
  for (let attempt = 0; attempt < 20 && !bb; attempt += 1) {
    await client.send('ControlSystem', {
      target: 'sensors',
      payload: { type: 'SetScienceTarget', data: { uuid } },
    });
    try {
      const handle = await client.page.waitForFunction(
        (wanted) => {
          const msgs = window.__messages || [];
          for (let i = msgs.length - 1; i >= 0; i -= 1) {
            const m = msgs[i];
            if (m.type !== 'BlackboardUpdate') continue;
            const entry = (m.data.updates || []).find(([sid]) => sid === 'sensor-radar');
            const data = entry && entry[1] && entry[1].data;
            if (data && data.selected_target === wanted) return data;
          }
          return null;
        },
        uuid,
        { timeout: 3_000 },
      );
      bb = await handle.jsonValue();
    } catch {
      /* the designation has not landed yet — re-issue and wait again */
    }
  }
  return bb;
}

// Push a live sensor-radar blackboard through the REAL client builder into the
// REAL shipped Sensors console, and hand back the scan card locator. Nothing on
// this path is hand-assembled: `buildSensorsConsoleState` derives the payload
// exactly as the client shell does, and `__updateConsole` is the seam the shell
// pushes it through.
//
// `gui/battleship/sensors.html` rather than the cruiser's `science.html` only
// because it takes the FLAT Sensors payload this builder returns, where Science
// takes the system-keyed station payload. Both mount the same
// `ph-sensor-panel` and both hand it the whole Sensors view unfiltered
// (gui/stations/science-console.js), so the row under test is the same row.
async function scanCardTextFor(consolePage, radarBlackboard) {
  const payload = await consolePage.evaluate(async (bb) => {
    const mod = await import('/gui/console-state.js');
    return mod.buildSensorsConsoleState({
      shipX: 0,
      shipZ: 0,
      shipYaw: 0,
      sensorsTarget: bb.selected_target,
      asteroids: [{ uuid: bb.selected_target, x: 300, z: -300, tags: ['ship'], name: 'TARGET' }],
      // The system-id -> blackboard-kind projection the host sends alongside
      // the blackboards themselves (gui/sim-state.js keeps it); the builder
      // selects by KIND, never by guessing at an id.
      blackboards: { 'sensor-radar': bb },
      blackboardKinds: { 'sensor-radar': 'SensorRadar' },
    });
  }, radarBlackboard);
  await consolePage.evaluate((p) => window.__updateConsole('sensors', p), payload);
  return consolePage.locator('ph-sensor-panel').locator('#scan-data');
}

test("a target that powered its weapons down reads WEAPONS COLD on another ship's Sensors", { tag: '@core' }, async ({
  context,
}) => {
  test.setTimeout(90_000);

  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: WEAPONS_COLD_WORLD }),
  );

  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);

  // Two seats: Helm to make the lobby a crew, Science because it owns the
  // sensors + sensor-radar systems on this hull.
  const helm = await createTestClient(context, hostId, { name: 'Helm' });
  const science = await createTestClient(context, hostId, { name: 'Sci' });

  await helm.send('SelectStation', { station: 'Helm' });
  await waitForStation(helm);
  await science.send('SelectStation', { station: 'Science' });
  await waitForStation(science);

  await helm.send('SetReady', { ready: true });
  await science.send('SetReady', { ready: true });
  await science.waitForMessage('GameStarted', 10_000);

  const worldSetup = await science.waitForMessage('WorldSetup', 5_000);
  expectFixtureWorld(worldSetup, WEAPONS_COLD_WORLD);

  await science.page.bringToFront();
  await science.page.waitForFunction(
    () => window.__messages?.some((m) => m.type === 'SimState'),
    undefined,
    { timeout: 15_000 },
  );

  const coldUuid = await uuidOfWorldEntity(science, 'cold-hauler');
  const armedUuid = await uuidOfWorldEntity(science, 'armed-hauler');
  expect(coldUuid, 'the cold hull must reach the client as a contact').toBeTruthy();
  expect(armedUuid, 'the armed hull must reach the client as a contact').toBeTruthy();

  // ── Ship A: weapons at 0 ────────────────────────────────────────────────
  const coldRadar = await designateAndReadRadar(science, coldUuid);
  expect(coldRadar, "Science must receive its ship's sensor-radar blackboard").not.toBeNull();
  expect(
    coldRadar.selected_target_weapons_cold,
    'a selected ship whose weapons group sits at level 0 reads cold',
  ).toBe(true);

  const consolePage = await context.newPage();
  await consolePage.goto('/gui/battleship/sensors.html');
  const coldScan = await scanCardTextFor(consolePage, coldRadar);
  await expect(coldScan).toContainText(ts('component.sensor_panel.weapons'));
  await expect(coldScan).toContainText(ts('component.sensor_panel.weapons_cold'));

  // ── The control: the same hull with its weapons up ──────────────────────
  const armedRadar = await designateAndReadRadar(science, armedUuid);
  expect(armedRadar, 'the second designation must publish too').not.toBeNull();
  expect(
    armedRadar.selected_target_weapons_cold,
    'the reading is looked up per selection, so an armed hull reads Some(false)',
  ).toBe(false);

  const armedScan = await scanCardTextFor(consolePage, armedRadar);
  await expect(armedScan).toContainText(ts('component.sensor_panel.weapons_powered'));

  await consolePage.close();
  await helm.close();
  await science.close();
});

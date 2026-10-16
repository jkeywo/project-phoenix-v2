// Sensors report the selected ship’s explicit restraint. Incoming power flow
// and a weapon’s charged local reserve do not imply a decision to hold fire.

import {
  test,
  expect,
  readHostPeerId,
  createTestClient,
  waitForWasmReady,
  expectFixtureWorld,
} from './fixtures';
import { ts } from './strings';

// Two otherwise identical stationary, explicitly targetable tugs; scenario
// restraint holds one. Accepted names feed real acquired identity samples.
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
overrides     = { tags = ["ship"], sensors_console = { long_range_radar = { range = 1200.0, shows = ["ship"], selects = ["ship"] } } }

# SHIP A — scenario explicitly holds weapons.
[[entity]]
template_path = "assets/entities/ship_harrow_tug.toml"
id            = "cold-hauler"
name          = "cold_hauler"
transform     = { position = [300.0, 0.0, -300.0] }
spawn_on      = "game_start"
overrides     = { name = "cold_hauler", display_name = "cold_hauler", tags = ["ship", "npc"], target = { tags = ["ship"] } }

# The control leaves its ordinary explicit restraint released.
[[entity]]
template_path = "assets/entities/ship_harrow_tug.toml"
id            = "armed-hauler"
name          = "armed_hauler"
transform     = { position = [-300.0, 0.0, -300.0] }
spawn_on      = "game_start"
overrides     = { name = "armed_hauler", display_name = "armed_hauler", tags = ["ship", "npc"], target = { tags = ["ship"] } }

[script]
setup = """
on_timer(1, "hold_fixture");
fn hold_fixture(ctx) { ctx.effects.hold_fire("cold_hauler"); }
"""
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

// Acquire the observer-local handle from the actual paid radar publication.
// WorldSetup deliberately carries no foreign live actor identity.
async function contactOfObservedName(client, name) {
  const handle = await client.page.waitForFunction((wanted) => {
    for (const message of [...(window.__messages || [])].reverse()) {
      if (message.type !== 'BlackboardUpdate') continue;
      const board = (message.data.updates || []).find(([owner]) => owner === 'sensor-radar')?.[1]?.data;
      const track = board?.tracks?.find((track) => !track.predicted && track.selectable && track.identity?.name === wanted);
      if (track) return track.contact_id;
    }
    return null;
  }, name, { timeout: 15000 });
  return handle.jsonValue();
}

// Designate `uuid` as the Science Target and wait for this ship's own
// sensor-radar blackboard to come back describing it. Returns that blackboard.
async function designateAndReadRadar(client, uuid, expectedCold) {
  let bb = null;
  for (let attempt = 0; attempt < 20 && !bb; attempt += 1) {
    await client.send('ControlSystem', {
      target: 'sensors',
      payload: { type: 'SetScienceTarget', data: { uuid } },
    });
    try {
      const handle = await client.page.waitForFunction(
        ({ wanted, expectedCold }) => {
          const msgs = window.__messages || [];
          for (let i = msgs.length - 1; i >= 0; i -= 1) {
            const m = msgs[i];
            if (m.type !== 'BlackboardUpdate') continue;
            const entry = (m.data.updates || []).find(([sid]) => sid === 'sensor-radar');
            const data = entry && entry[1] && entry[1].data;
            const track = data?.tracks?.find(candidate => candidate.contact_id === wanted);
            if (data?.selected_target === wanted && data.selected_target_weapons_cold === expectedCold
              && track?.selectable && track.predicted === false
              && track.assessment?.weapons_restrained === expectedCold
              && Number.isSafeInteger(track.assessment_tick) && track.assessment_tick >= 0
              && Number.isSafeInteger(track.observed_tick) && track.observed_tick >= track.assessment_tick) return data;
          }
          return null;
        },
        { wanted: uuid, expectedCold },
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
async function scanCardFor(consolePage, radarBlackboard, expectedRestraint) {
  const track = radarBlackboard.tracks.find(candidate => candidate.contact_id === radarBlackboard.selected_target);
  expect(track.assessment.weapons_restrained).toBe(expectedRestraint);
  expect(track.assessment_tick).toBeGreaterThanOrEqual(0);
  expect(track.assessment_tick).toBeLessThanOrEqual(track.observed_tick);
  const payload = await consolePage.evaluate(async (bb) => {
    const mod = await import('/gui/console-state.js');
    return mod.buildSensorsConsoleState({
      shipX: 0,
      shipZ: 0,
      shipYaw: 0,
      sensorsTarget: bb.selected_target,
      asteroids: [], // This builder must use measured records, not world truth.
      // The system-id -> blackboard-kind projection the host sends alongside
      // the blackboards themselves (gui/sim-state.js keeps it); the builder
      // selects by KIND, never by guessing at an id.
      blackboards: { 'sensor-radar': bb },
      blackboardKinds: { 'sensor-radar': 'SensorRadar' },
    });
  }, radarBlackboard);
  expect(payload.target_uuid).toBe(track.contact_id);
  expect(payload.target_track.id).toBe(track.contact_id);
  expect(payload.target_restrained).toBe(expectedRestraint);
  expect(payload.target_elevation.sample_tick).toBe(track.observed_tick);
  await consolePage.evaluate((p) => window.__updateConsole('sensors', JSON.stringify(p)), payload);
  const panel = consolePage.locator('ph-sensor-panel');
  await expect(panel.locator('#target-name')).toHaveText(track.identity.name);
  const scan = panel.locator('#scan-data');
  const restraint = scan.locator('.scan-row').filter({
    has: consolePage.getByText(ts('sensing.track.restrained'), { exact: true }),
  });
  await expect(restraint).toHaveCount(1);
  await expect(restraint.locator('.k')).toHaveText(ts('sensing.track.restrained'));
  await expect(restraint.locator('.v')).toHaveText(ts(expectedRestraint ? 'sensing.track.yes' : 'sensing.track.no'));
  const altitude = scan.locator('.scan-row').filter({
    has: consolePage.getByText(ts('sensing.track.elevation.title'), { exact: true }),
  });
  await expect(altitude).toHaveCount(1);
  await expect(altitude.locator('.v')).toHaveText(
    ts('sensing.track.elevation.value', { height: track.elevation.height, error: track.elevation.error })
      + ' \u00b7 ' + ts('sensing.track.elevation.sample', { tick: track.observed_tick }),
  );
}

test("Sensors displays each target's dated observed weapons restraint", { tag: '@core' }, async ({
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

  expect(worldSetup.data.world.entities.some((entity) => entity.id === 'cold-hauler' || entity.id === 'armed-hauler')).toBe(false);
  const coldUuid = await contactOfObservedName(science, 'cold_hauler');
  const armedUuid = await contactOfObservedName(science, 'armed_hauler');
  expect(coldUuid, 'the cold hull must reach the client as a contact').toBeTruthy();
  expect(armedUuid, 'the armed hull must reach the client as a contact').toBeTruthy();

  // ── Ship A: explicit restraint ────────────────────────────────────────────────
  const coldRadar = await designateAndReadRadar(science, coldUuid, true);
  expect(coldRadar, "Science must receive its ship's sensor-radar blackboard").not.toBeNull();
  expect(
    coldRadar.selected_target_weapons_cold,
    'a selected ship with explicit restraint reads cold',
  ).toBe(true);

  const consolePage = await context.newPage();
  await consolePage.goto('/gui/battleship/sensors.html');
  await scanCardFor(consolePage, coldRadar, true);

  // ── The control: the same hull with its weapons up ──────────────────────
  const armedRadar = await designateAndReadRadar(science, armedUuid, false);
  expect(armedRadar, 'the second designation must publish too').not.toBeNull();
  expect(
    armedRadar.selected_target_weapons_cold,
    'the reading is looked up per selection, so an armed hull reads Some(false)',
  ).toBe(false);

  expect(armedUuid).not.toBe(coldUuid);
  await scanCardFor(consolePage, armedRadar, false);

  await consolePage.close();
  await helm.close();
  await science.close();
});

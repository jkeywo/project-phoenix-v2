// Issue #1167 (S13) — Smoke test: Tactical lock -> Engineering tractor -> Helm
// feels the weight, on real consoles.
//
// The browser-level half of the marquee "a silent crew completes the ship's
// external work" proof (the headless half is
// `a_full_ai_crew_completes_a_tow_a_transfer_and_a_field_repair` in
// tests/headless_runner.rs). Here three HUMAN seats drive the chain on the real
// WASM host over the actual wire, each sending exactly the ControlSystem envelope
// its console's control sends (gui/action-map.js):
//
//   1. TACTICAL locks the derelict — SetTarget to the tactical-radar system.
//   2. ENGINEERING engages the tractor — EngageTractor to the tractor system.
//   3. HELM feels the weight — the `tractor` blackboard reaches the helm carrying
//      the coupled target, which is exactly what the helm console's under-tow-load
//      indicator (issue #1157, gui/console-state.js `buildHelmTowLoadView`) keys
//      on. The chain is observable end to end without naming a single authored
//      gameplay number.
//
// This runs under the same Playwright harness as the other smokes (WASM host +
// PeerJS-shimmed message clients), so it is GATE/CI work, not the cheap vitest
// pass. The `createTestClient` shim is a raw message client, so "real consoles"
// means the real SERVER consoles/admission/hosts driven by the real console wire
// protocol — the same standard every smoke here holds.

import {
  test,
  expect,
  readHostPeerId,
  createTestClient,
  waitForWasmReady,
  expectFixtureWorld,
} from './fixtures';

// A self-contained world served in place of default.toml: the shipped Alliance
// Destroyer as the player ship (the one hull that carries a tractor) and one
// heavy derelict inside its tractor reach. The destroyer's tactical radar is
// widened here so a human Tactical can lock the non-hostile derelict at a working
// tractor distance (the shipped 50-unit horizon is a close-combat range); the
// override is the world's, re-applied onto the picked hull, never a change to the
// shipped destroyer (server_app::player_hull_config).
const TRACTOR_WORLD = `
[global]
seed = 1167
title = "Operations Tractor Fixture"

[ambient_light]
color      = [0.6, 0.55, 0.5]
brightness = 300.0

[[entity]]
template_path = "assets/entities/alliance_destroyer.toml"
id = "player-ship"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"
overrides = { weapons_console = { radar = { range = 700.0 } } }

# The derelict — a heavy neutral hauler 120 units to starboard, well inside the
# destroyer's 500-unit tractor reach. Its heavy mass makes the tow real work.
[[entity]]
template_path = "assets/entities/ship_civilian_hauler.toml"
name = "world.smoke_operations.entity.derelict.name"
transform = { position = [120.0, 0.0, 0.0] }
overrides = { mass = 240000.0 }
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

// The `data` half of this ship's blackboard for `systemId`, taken from the most
// recent BlackboardUpdate that carried it. Every non-repair blackboard is a
// Target::All broadcast (server_app::broadcast_blackboard_updates), so any client
// receives it; the entry shape is `[systemId, { kind, data }]` (gui/sim-state.js).
async function latestBlackboard(client, systemId) {
  return client.page.evaluate((id) => {
    const msgs = window.__messages || [];
    for (let i = msgs.length - 1; i >= 0; i -= 1) {
      const m = msgs[i];
      if (m.type !== 'BlackboardUpdate') continue;
      const entry = (m.data.updates || []).find(([sid]) => sid === id);
      if (entry && entry[1] && entry[1].data) return entry[1].data;
    }
    return null;
  }, systemId);
}

test('Tactical lock -> Engineering tractor -> Helm feels the weight', async ({
  context,
}) => {
  test.setTimeout(60_000);

  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: TRACTOR_WORLD }),
  );

  const serverPage = await context.newPage();
  const serverCrashes = [];
  serverPage.on('crash', () => serverCrashes.push('server page crashed'));
  serverPage.on('pageerror', (err) => serverCrashes.push(err.message));

  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);

  // Three human seats: Tactical locks, Engineering engages, Helm observes.
  const tactical = await createTestClient(context, hostId, { name: 'Tac' });
  const engineer = await createTestClient(context, hostId, { name: 'Eng' });
  const helm = await createTestClient(context, hostId, { name: 'Helm' });

  await tactical.send('SelectStation', { station: 'Tactical' });
  await waitForStation(tactical);
  await engineer.send('SelectStation', { station: 'Engineering' });
  await waitForStation(engineer);
  await helm.send('SelectStation', { station: 'Helm' });
  await waitForStation(helm);

  await tactical.send('SetReady', { ready: true });
  await engineer.send('SetReady', { ready: true });
  await helm.send('SetReady', { ready: true });
  await tactical.waitForMessage('GameStarted', 10_000);
  await engineer.waitForMessage('GameStarted', 10_000);
  await helm.waitForMessage('GameStarted', 10_000);

  // The derelict's uuid, from the WorldSetup the host sends every client.
  const worldSetup = await tactical.waitForMessage('WorldSetup', 5_000);
  expectFixtureWorld(worldSetup, TRACTOR_WORLD);
  const entities = worldSetup?.data?.world?.entities ?? [];
  const derelict = entities.find(
    (e) => Array.isArray(e.tags) && e.tags.includes('civilian'),
  );
  expect(derelict, 'the derelict must appear in WorldSetup').toBeDefined();
  const derelictUuid = derelict.uuid;

  // Let the sim reach a running state so admission is live.
  await tactical.page.bringToFront();
  await tactical.page.waitForFunction(
    () => window.__messages?.some((m) => m.type === 'SimState'),
    undefined,
    { timeout: 15_000 },
  );

  // 1. TACTICAL locks the derelict (the real set_target wire path).
  await tactical.send('ControlSystem', {
    target: 'tactical-radar',
    payload: { type: 'SetTarget', data: { uuid: derelictUuid } },
  });

  // 2. ENGINEERING engages the tractor against that lock (the real engage_tractor
  //    wire path). Retried a few times: the lock and the engage are drained on
  //    different ticks, and a throttled headless server may not have admitted the
  //    lock before the first engage arrives.
  await engineer.page.bringToFront();
  await helm.page.bringToFront();
  let held = false;
  for (let attempt = 0; attempt < 20 && !held; attempt += 1) {
    await engineer.send('ControlSystem', {
      target: 'tractor',
      payload: { type: 'EngageTractor' },
    });
    // 3. HELM feels the weight: wait for a `tractor` blackboard that names the
    //    coupled derelict — the exact gate the under-tow-load indicator reads.
    try {
      await helm.page.waitForFunction(
        (id) => {
          const msgs = window.__messages || [];
          for (let i = msgs.length - 1; i >= 0; i -= 1) {
            const m = msgs[i];
            if (m.type !== 'BlackboardUpdate') continue;
            const entry = (m.data.updates || []).find(([sid]) => sid === 'tractor');
            const bb = entry && entry[1] && entry[1].data;
            if (bb && bb.coupled_target === id) return true;
          }
          return false;
        },
        derelictUuid,
        { timeout: 2_500 },
      );
      held = true;
    } catch {
      /* not yet — re-issue the engage and wait again */
    }
  }

  const helmTractor = await latestBlackboard(helm, 'tractor');
  expect(
    helmTractor,
    'the helm must receive the tractor blackboard (the under-tow-load indicator reads it)',
  ).not.toBeNull();
  expect(
    helmTractor.coupled_target,
    'the helm must feel the weight: the tractor blackboard names the coupled derelict',
  ).toBe(derelictUuid);

  // The engineering seat that engaged sees the same coupled hold — the chain is
  // consistent across the two seats.
  const engTractor = await latestBlackboard(engineer, 'tractor');
  expect(engTractor?.coupled_target).toBe(derelictUuid);
  expect(engTractor?.engaged).toBe(true);

  expect(serverCrashes, `server errors: ${serverCrashes.join('; ')}`).toEqual([]);

  await tactical.close();
  await engineer.close();
  await helm.close();
});

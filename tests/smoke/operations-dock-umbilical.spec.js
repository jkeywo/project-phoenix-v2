// Issue #1167 (S13) — Smoke test: Helm dock -> Engineering umbilical -> capacity
// moves between two hulls, on real consoles.
//
// The second browser-level half of the marquee "a silent crew completes the
// ship's external work" proof. Two HUMAN seats drive the transfer chain on the
// real WASM host over the actual wire, each sending exactly the ControlSystem
// envelope its console's control sends (gui/action-map.js):
//
//   1. HELM docks the two hulls — Dock to the dock system. The server flies the
//      mate manoeuvre onto the nearest viable dock-marker pair.
//   2. ENGINEERING runs the umbilical once docked — StartTransfer to the umbilical
//      system. The flow gates on the dock the Helm achieved.
//   3. CAPACITY MOVES between the two hulls — the `umbilical` blackboard shows the
//      flow running and the docked partner's reserve_fuel ledger filling from the
//      operator's own (gui/console-state.js `buildUmbilicalConsoleState`).
//
// See operations-tractor-helm.spec.js for the harness note on why sending the
// console wire messages IS driving the real consoles here.

import {
  test,
  expect,
  readHostPeerId,
  createTestClient,
  waitForWasmReady,
  expectFixtureWorld,
} from './fixtures';

// Two self-contained worlds served in place of default.toml, one per hull that
// carries both a dock and an umbilical: the shipped Alliance Destroyer (since
// #1159/#1160) and the Alliance Cruiser (since #1388/#1390). Each puts its hull
// at the origin with one passive umbilical berth drawn up close alongside,
// inside the dock range. Three overrides, all the world's own intent re-applied
// onto the picked hull (server_app::player_hull_config), never a change to the
// shipped entity:
//   * a reserve_fuel ledger, the source the umbilical delivers from (the destroyer
//     authors none of its own by design — a ledger would fold into every world);
//   * a brisk dock approach speed and umbilical rate, so the mate forms and the
//     capacity moves inside a throttled headless-browser run.
const DOCK_WORLD = `
[global]
seed = 1167
title = "Operations Dock Fixture"

[ambient_light]
color      = [0.6, 0.55, 0.5]
brightness = 300.0

# Auto-select the destroyer as the player hull (single [[available_ships]] entry
# takes server.html's 'auto-select' branch). Without it the host takes the
# 'legacy-fallback' branch and station-gate-checks a hardcoded
# alliance_cruiser.toml whose include closure this world never preloads, faulting
# boot before __wasmReady — see comms-visiting-station.spec.js's DESTROYER_WORLD
# for the same note.
[[available_ships]]
template_path = "assets/entities/alliance_destroyer.toml"

[[entity]]
template_path = "assets/entities/alliance_destroyer.toml"
id = "player-ship"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"
overrides = { dock = { approach_speed = 120.0 }, umbilical = { rate = 40.0 }, infrastructure = { condition_max = 100.0, decay_per_sec = 0.0, hull_damage_share = 0.0, capacity = [{ id = "reserve_fuel", amount = 100, ceiling = 100 }] } }

# The berth — a passive umbilical berth 70 units to starboard, inside the
# destroyer's 200-unit dock range, its reserve_fuel ledger empty with headroom.
# Spawned at game_start (like the player ship) rather than at world load: the
# berth's dock markers come from its dock_probe model-variant rig sidecar, and
# only a game_start spawn is gated behind the browser asset preload that delivers
# that sidecar first. A world-load spawn races the async sidecar fetch and can
# come up with no DockMarkers (and there is no re-resolve), leaving the berth
# permanently un-dockable in the browser host. The native config cache reads the
# sidecar synchronously, so this only bites the WASM smoke.
[[entity]]
template_path = "assets/entities/umbilical_berth.toml"
name = "world.smoke_operations.entity.depot.name"
transform = { position = [70.0, 0.0, 0.0] }
spawn_on = "game_start"
`;

// The cruiser's own case (issue #1390). Same berth, same overrides, same
// distances — only the hull differs. This hull authors no `[infrastructure]`
// ledger either, for the same reason the destroyer does not, so the world
// supplies the reserve_fuel source the transfer draws from.
const CRUISER_DOCK_WORLD = `
[global]
seed = 1390
title = "Cruiser Operations Dock Fixture"

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
overrides = { dock = { approach_speed = 120.0 }, umbilical = { rate = 40.0 }, infrastructure = { condition_max = 100.0, decay_per_sec = 0.0, hull_damage_share = 0.0, capacity = [{ id = "reserve_fuel", amount = 100, ceiling = 100 }] } }

[[entity]]
template_path = "assets/entities/umbilical_berth.toml"
name = "world.smoke_operations.entity.depot.name"
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

// The `data` half of this ship's blackboard for `systemId`, from the most recent
// BlackboardUpdate that carried it (Target::All broadcast; entry shape
// `[systemId, { kind, data }]`, gui/sim-state.js).
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

// The whole chain, once, against one hull's world. Both cases below run it —
// the destroyer's (#1167) and the cruiser's (#1390) — because the claim is that
// the SAME two seats, the SAME two wire messages and the SAME blackboard gates
// carry an authored dock+umbilical pair on any hull that mounts them.
async function runTransferChain(context, world) {
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: world }),
  );

  const serverPage = await context.newPage();
  const serverCrashes = [];
  serverPage.on('crash', () => serverCrashes.push('server page crashed'));
  serverPage.on('pageerror', (err) => serverCrashes.push(err.message));

  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);

  // Two human seats: Helm owns the dock, Engineering owns the umbilical.
  const helm = await createTestClient(context, hostId, { name: 'Helm' });
  const engineer = await createTestClient(context, hostId, { name: 'Eng' });

  await helm.send('SelectStation', { station: 'Helm' });
  await waitForStation(helm);
  await engineer.send('SelectStation', { station: 'Engineering' });
  await waitForStation(engineer);

  await helm.send('SetReady', { ready: true });
  await engineer.send('SetReady', { ready: true });
  await helm.waitForMessage('GameStarted', 10_000);
  await engineer.waitForMessage('GameStarted', 10_000);

  const worldSetup = await helm.waitForMessage('WorldSetup', 5_000);
  expectFixtureWorld(worldSetup, world);

  await helm.page.bringToFront();
  await engineer.page.bringToFront();
  await helm.page.waitForFunction(
    () => window.__messages?.some((m) => m.type === 'SimState'),
    undefined,
    { timeout: 15_000 },
  );

  // Wait until the berth is in dock range — the helm's dock blackboard reports it
  // `available` — before issuing the dock, so the manoeuvre has a target to fly to.
  await helm.page.waitForFunction(
    () => {
      const msgs = window.__messages || [];
      for (let i = msgs.length - 1; i >= 0; i -= 1) {
        const m = msgs[i];
        if (m.type !== 'BlackboardUpdate') continue;
        const entry = (m.data.updates || []).find(([sid]) => sid === 'dock');
        const bb = entry && entry[1] && entry[1].data;
        if (bb && bb.available) return true;
      }
      return false;
    },
    undefined,
    { timeout: 20_000 },
  );

  // 1. HELM docks (the real dock wire path). Re-issued until the two hulls mate:
  //    the manoeuvre takes several ticks and a throttled server may drop the first.
  let docked = false;
  for (let attempt = 0; attempt < 30 && !docked; attempt += 1) {
    await helm.send('ControlSystem', { target: 'dock', payload: { type: 'Dock' } });
    try {
      await helm.page.waitForFunction(
        () => {
          const msgs = window.__messages || [];
          for (let i = msgs.length - 1; i >= 0; i -= 1) {
            const m = msgs[i];
            if (m.type !== 'BlackboardUpdate') continue;
            const entry = (m.data.updates || []).find(([sid]) => sid === 'dock');
            const bb = entry && entry[1] && entry[1].data;
            if (bb && bb.docked) return true;
          }
          return false;
        },
        undefined,
        { timeout: 3_000 },
      );
      docked = true;
    } catch {
      /* still flying the mate — hold the dock intent and wait again */
    }
  }
  const dockBb = await latestBlackboard(helm, 'dock');
  expect(dockBb?.docked, 'the two hulls must reach a mated dock').toBe(true);

  // 2. ENGINEERING runs the umbilical (the real start_transfer wire path), and
  // 3. CAPACITY MOVES: wait for an umbilical blackboard that is running and whose
  //    docked partner's level (the berth's reserve_fuel) has risen above empty.
  let flowed = false;
  for (let attempt = 0; attempt < 30 && !flowed; attempt += 1) {
    await engineer.send('ControlSystem', {
      target: 'umbilical',
      payload: { type: 'StartTransfer' },
    });
    try {
      await engineer.page.waitForFunction(
        () => {
          const msgs = window.__messages || [];
          for (let i = msgs.length - 1; i >= 0; i -= 1) {
            const m = msgs[i];
            if (m.type !== 'BlackboardUpdate') continue;
            const entry = (m.data.updates || []).find(([sid]) => sid === 'umbilical');
            const bb = entry && entry[1] && entry[1].data;
            if (bb && bb.running && (bb.partner_level ?? 0) > 0) return true;
          }
          return false;
        },
        undefined,
        { timeout: 4_000 },
      );
      flowed = true;
    } catch {
      /* not flowing yet — re-issue and wait again */
    }
  }

  const umbBb = await latestBlackboard(engineer, 'umbilical');
  expect(umbBb, 'engineering must receive the umbilical blackboard').not.toBeNull();
  expect(umbBb.running, 'the umbilical must be flowing across the mated dock').toBe(true);
  // Capacity has crossed the dock into the partner hull's ledger, and left the
  // operator's own — a real move between two hulls, not a spawn.
  expect(
    umbBb.partner_level,
    'the docked partner (the berth) must have received capacity across the umbilical',
  ).toBeGreaterThan(0);
  expect(
    umbBb.operator_level,
    "the capacity came out of the ship's own ledger",
  ).toBeLessThan(100);

  expect(serverCrashes, `server errors: ${serverCrashes.join('; ')}`).toEqual([]);

  await helm.close();
  await engineer.close();
}

test('Helm dock -> Engineering umbilical -> capacity moves between two hulls', { tag: '@core' }, async ({
  context,
}) => {
  test.setTimeout(90_000);
  await runTransferChain(context, DOCK_WORLD);
});

// Issue #1390: the cruiser authors the same umbilical over the dock it gained in
// #1388, so it completes the same transfer. Nothing below the TOML is
// hull-aware, and this case is what says so.
//
// NOT `@core`: the breadth pass already runs this chain once, above, and this is
// the per-hull variant of it — the same call #1388's own cruiser dock case made
// in helm-dock.spec.js. It runs on the nightly, on workflow_dispatch, and on a
// PR labelled `smoke-full`.
test('cruiser: Helm dock -> Engineering umbilical -> capacity moves between two hulls', async ({
  context,
}) => {
  test.setTimeout(90_000);
  await runTransferChain(context, CRUISER_DOCK_WORLD);
});

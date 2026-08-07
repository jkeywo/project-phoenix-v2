// Issue #475 — Smoke test: combat_test.toml wave scenario.
//
// Verifies the full combat-test scenario boots correctly:
//   - The ship picker offers one card per authored hull
//   - A station entity is spawned (the defeat condition requires one)
//   - Every objective the world adds on `on_world_loaded` reaches the client
//   - The undelayed wave spawns shortly after game start, and its own
//     objective arrives with it. Since issue #892 that is the ONLY timed wave —
//     every later one hangs off `on_all_destroyed` over the previous one's
//     group — so it is the only wave a fast smoke run can see.
//
// Combat balance / wave timing / victory conditions are NOT tested
// here — they're too time-sensitive for fast smoke runs. The unit
// tests in src/world/config.rs and src/world/server.rs cover the
// trigger-evaluation invariants.

import {
  test,
  expect,
  readHostPeerId,
  createTestClient,
  waitForWasmReady,
  stripHeavyEntities,
  countTableArray,
} from './fixtures';
import fs from 'fs';
import path from 'path';

const COMBAT_TEST_TOML = fs.readFileSync(
  path.join(__dirname, '../../assets/worlds/combat_test.toml'),
  'utf-8',
);

// ── Expectations read out of the world, not pinned here (issue #941) ─────────
//
// This spec deliberately boots the *shipped* combat_test world — it is the demo
// scenario, and "does it still boot end to end" is the thing being smoked, so a
// fixture world would test something else. That makes every authored figure in
// it a moving target, so nothing authored is written down twice: the ship-card
// count and the world-load objective ids are read from the same text the spec
// serves to the page. Each assertion then means "the pipeline delivered what the
// world authored", which still fails if a wave, an objective or a hull is
// dropped on the way through, but not when a designer adds one.
//
// A spec that does NOT need the shipped world should use a self-contained
// fixture instead — see the header of `fixtures.ts` for the convention.
const AVAILABLE_SHIP_COUNT = countTableArray(COMBAT_TEST_TOML, 'available_ships');

/** Objective ids added by the `[[trigger]]` blocks `keep` selects. */
function objectivesAddedBy(toml: string, keep: (block: string) => boolean): string[] {
  const ids: string[] = [];
  for (const block of toml.split(/^\[\[trigger\]\]\s*$/m).slice(1)) {
    if (!keep(block)) continue;
    for (const action of block.split(/^\s*\[\[trigger\.action\]\]\s*$/m)) {
      if (!/^\s*type\s*=\s*"add_objective"/m.test(action)) continue;
      const id = action.match(/^\s*id\s*=\s*"([^"]+)"/m);
      if (id) ids.push(id[1]);
    }
  }
  return ids;
}

const WORLD_LOAD_OBJECTIVES = objectivesAddedBy(COMBAT_TEST_TOML, (b) =>
  /^\s*condition\s*=\s*"on_world_loaded"/m.test(b),
);

// The undelayed wave: `on_timer` with `after_secs = 0`. Since #892 it is the
// only timed wave — every later one hangs off `on_all_destroyed` over the
// previous one's group — so it is the only wave a fast smoke run can see.
const FIRST_WAVE_OBJECTIVES = objectivesAddedBy(
  COMBAT_TEST_TOML,
  (b) =>
    /^\s*condition\s*=\s*"on_timer"/m.test(b) && /^\s*after_secs\s*=\s*0(\.0+)?\s*$/m.test(b),
);

test('combat_test scenario: starbase + objective + player + first wave appear after game start', async ({ context }) => {
  await context.route('**/assets/worlds/combat_test.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: stripHeavyEntities(COMBAT_TEST_TOML) }),
  );

  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/combat_test.toml');

  // combat_test.toml declares more than one [[available_ships]], so
  // finishInit() re-shows the scenario-panel as a ship picker rather than
  // calling startServer() immediately. Since 3d49287f the picker is the
  // <ph-ship-picker> web component (rendering .ship-card elements in its
  // shadow root, dispatching a "ship-selected" event on click) rather than
  // plain button.world-btn elements. Wait for the first ship card to appear
  // and click it so wasm_init() runs and __wasmReady can fire.
  await serverPage.waitForSelector('#scenario-panel ph-ship-picker .ship-card', {
    state: 'visible',
    timeout: 60_000,
  });
  // One card per authored hull — the count comes from the world TOML this test
  // is serving, so adding a hull to the roster does not break the spec while
  // dropping one on the way to the picker still does.
  await expect(serverPage.locator('#scenario-panel ph-ship-picker .ship-card'))
    .toHaveCount(AVAILABLE_SHIP_COUNT);
  // First card is the world's first authored hull — a crewed one, which is
  // what this test needs to drive a Captain station.
  await serverPage.click('#scenario-panel ph-ship-picker .ship-card:first-child');

  await waitForWasmReady(serverPage);

  const hostId = await readHostPeerId(serverPage);

  // Boot a single client and take the captain station so we can start the game.
  const captain = await createTestClient(context, hostId, { name: 'Cap' });
  await captain.send('SelectStation', { station: 'Captain' });
  await captain.page.waitForFunction(
    (t) => (window as any).__messages?.some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t,
    ),
    captain.token,
    { timeout: 5_000 },
  );

  await captain.send('SetReady', { ready: true });
  await captain.waitForMessage('GameStarted', 10_000);

  // After game start the WorldSetup contains the static entities:
  // starbase + asteroid fields + planet + star.
  const worldSetupMsg = await captain.waitForMessage('WorldSetup', 5_000) as any;
  const entities: any[] = worldSetupMsg?.data?.world?.entities ?? [];

  // A station entity must be present (defeat condition requires one).
  const starbase = entities.find(
    (e: any) => Array.isArray(e.tags) && e.tags.includes('station'),
  );
  expect(
    starbase,
    `Expected a station entity in WorldSetup. Got: ${
      JSON.stringify(entities.map((e: any) => ({ id: e.id, tags: e.tags })))
    }`,
  ).toBeDefined();

  // Every objective the world adds on `on_world_loaded` must fire as part of
  // the initial ObjectivesUpdate broadcast. The ids come from the world TOML,
  // not from a list written down here.
  expect(
    WORLD_LOAD_OBJECTIVES.length,
    'combat_test.toml adds no on_world_loaded objective — nothing to assert',
  ).toBeGreaterThan(0);
  const objMsg = await captain.waitForMessage('ObjectiveSummary', 5_000) as any;
  const objectives: any[] = objMsg?.data?.objectives ?? [];
  const delivered = objectives.map((o: any) => o.id);
  for (const id of WORLD_LOAD_OBJECTIVES) {
    expect(
      delivered,
      `Expected the world-load objective ${id} after game start. Got: ${JSON.stringify(objectives)}`,
    ).toContain(id);
  }

  // Wave 1 spawn (on_timer after_secs = 0) should fire promptly. Wait a few
  // ticks for the spawn to register and the EntitySpawned message to
  // broadcast. That trigger's own objective is added undelayed alongside it —
  // only waves 2..8 carry the ten-second breather.
  expect(
    FIRST_WAVE_OBJECTIVES.length,
    'combat_test.toml has no undelayed on_timer wave — nothing a fast smoke run can see',
  ).toBeGreaterThan(0);
  await captain.waitForMessage('EntitySpawned', 5_000);
  for (const id of FIRST_WAVE_OBJECTIVES) {
    await captain.page.waitForFunction(
      (waveId) => (window as any).__messages?.some(
        (m: any) => m.type === 'ObjectiveSummary'
          && Array.isArray(m.data?.objectives)
          && m.data.objectives.some((o: any) => o.id === waveId),
      ),
      id,
      { timeout: 5_000 },
    );
  }

  await captain.close();
});

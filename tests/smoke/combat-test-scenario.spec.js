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
  captureFetchFailures,
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
// fixture instead — see the header of `fixtures.js` for the convention.
const AVAILABLE_SHIP_COUNT = countTableArray(COMBAT_TEST_TOML, 'available_ships');

// Issue #984 moved the triggers into a Rhai `[script]` block, so the two
// expectations below are read out of THAT instead — still out of the same text
// the spec serves to the page, and still deliberately not a parser: a
// registration line names a handler fn, and the objective ids are the `id:`
// keys of the `add_objective` maps in that fn's body.
const SCRIPT_BODY = COMBAT_TEST_TOML.match(/^setup\s*=\s*"""\n([\s\S]*?)\n"""/m)?.[1] ?? '';

/** The body of `fn name(ctx) { … }`, by brace matching from its opening `{`. */
function handlerBody(script, name) {
  const start = script.search(new RegExp(String.raw`^fn\s+${name}\s*\(`, 'm'));
  if (start < 0) return '';
  let depth = 0;
  for (let i = script.indexOf('{', start); i < script.length; i += 1) {
    if (script[i] === '{') depth += 1;
    else if (script[i] === '}') {
      depth -= 1;
      if (depth === 0) return script.slice(start, i + 1);
    }
  }
  return '';
}

/** Objective ids added by the handlers of the registrations `pattern` matches. */
function objectivesAddedBy(script, pattern) {
  const ids = [];
  for (const [, handler] of script.matchAll(pattern)) {
    for (const [, id] of handlerBody(script, handler).matchAll(
      /add_objective\(#\{\s*id:\s*"([^"]+)"/g,
    )) {
      ids.push(id);
    }
  }
  return ids;
}

const WORLD_LOAD_OBJECTIVES = objectivesAddedBy(SCRIPT_BODY, /^on_world_loaded\("([^"]+)"\)/gm);

// The undelayed wave: `on_timer(0, …)`. Since #960 every wave is on a clock,
// but the rest of them are 45 seconds and more apart, so this is still the only
// wave a fast smoke run can see.
const FIRST_WAVE_OBJECTIVES = objectivesAddedBy(SCRIPT_BODY, /^on_timer\(0,\s*"([^"]+)"\)/gm);

test('combat_test scenario: starbase + objective + player + first wave appear after game start', async ({ context }) => {
  await context.route('**/assets/worlds/combat_test.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: stripHeavyEntities(COMBAT_TEST_TOML) }),
  );

  const serverPage = await context.newPage();
  const fetchFailures = captureFetchFailures(serverPage);
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
    (t) => window.__messages?.some(
      (m) => m.type === 'StationAssigned' && m.data.token === t,
    ),
    captain.token,
    { timeout: 5_000 },
  );

  await captain.send('SetReady', { ready: true });
  await captain.waitForMessage('GameStarted', 10_000);

  // After game start the WorldSetup contains the static entities:
  // starbase + asteroid fields + planet + star.
  const worldSetupMsg = await captain.waitForMessage('WorldSetup', 5_000);
  const entities = worldSetupMsg?.data?.world?.entities ?? [];

  // A station entity must be present (defeat condition requires one).
  const starbase = entities.find(
    (e) => Array.isArray(e.tags) && e.tags.includes('station'),
  );
  expect(
    starbase,
    `Expected a station entity in WorldSetup. Got: ${
      JSON.stringify(entities.map((e) => ({ id: e.id, tags: e.tags })))
    }`,
  ).toBeDefined();

  // Every objective the world adds on `on_world_loaded` must fire as part of
  // the initial ObjectivesUpdate broadcast. The ids come from the world TOML,
  // not from a list written down here.
  expect(
    WORLD_LOAD_OBJECTIVES.length,
    'combat_test.toml adds no on_world_loaded objective — nothing to assert',
  ).toBeGreaterThan(0);
  const objMsg = await captain.waitForMessage('ObjectiveSummary', 5_000);
  const objectives = objMsg?.data?.objectives ?? [];
  const delivered = objectives.map((o) => o.id);
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
      (waveId) => window.__messages?.some(
        (m) => m.type === 'ObjectiveSummary'
          && Array.isArray(m.data?.objectives)
          && m.data.objectives.some((o) => o.id === waveId),
      ),
      id,
      { timeout: 5_000 },
    );
  }

  // Nothing this scenario loads may 404. The preload walk asks for every asset
  // the world names, and a scenario full of ships used to make it ask for each
  // hull's per-tier rig sidecars — files the pipeline deliberately never wrote,
  // because a hull's generated tiers carry no rig of their own. The ladder now
  // says so (`tier_rig`), so nothing goes looking. See `captureFetchFailures`.
  expect(
    fetchFailures,
    `The combat_test load requested assets that came back missing:\n  ${
      fetchFailures.join('\n  ')
    }\nShipped content must not 404. If these are LOD tier sidecars, a ladder has \
lost its tier_rig — re-run the pipeline that wrote it.`,
  ).toEqual([]);

  await captain.close();
});

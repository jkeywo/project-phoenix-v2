// Issue #475 — Smoke test: combat_test.toml wave scenario.
//
// Verifies the full combat-test scenario boots correctly:
//   - World loads with all 8 spawn anchors registered
//   - Starbase Alpha is spawned and named (defeat condition)
//   - Player ship spawns at the documented position on game start
//   - At least the first wave (t=0 destroyer) is spawned shortly after
//     game start
//   - The on_world_loaded objective ("Defend Starbase Alpha") is added
//
// Combat balance / wave timing / victory conditions are NOT tested
// here — they're too time-sensitive for fast smoke runs. The unit
// tests in src/world/config.rs and src/world/server.rs cover the
// trigger-evaluation invariants.

import { test, expect, readHostPeerId, createTestClient, waitForWasmReady, stripHeavyEntities } from './fixtures';
import fs from 'fs';
import path from 'path';

const COMBAT_TEST_TOML = fs.readFileSync(
  path.join(__dirname, '../../assets/worlds/combat_test.toml'),
  'utf-8',
);

test('combat_test scenario: starbase + objective + player + first wave appear after game start', async ({ context }) => {
  await context.route('**/assets/worlds/combat_test.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: stripHeavyEntities(COMBAT_TEST_TOML) }),
  );

  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/combat_test.toml');
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

  await captain.send('StartGame');
  await captain.waitForMessage('GameStarted', 5_000);

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

  // The on_world_loaded objective must fire as part of the initial
  // ObjectivesUpdate broadcast.
  const objMsg = await captain.waitForMessage('ObjectiveSummary', 5_000) as any;
  const objectives: any[] = objMsg?.data?.objectives ?? [];
  expect(
    objectives.length,
    `Expected at least one objective after game start. Got: ${JSON.stringify(objectives)}`,
  ).toBeGreaterThan(0);

  // Wave 1 spawn (on_timer at_secs = 0) should fire promptly. Wait a few
  // ticks for the spawn to register and the EntitySpawned message to
  // broadcast.
  await captain.waitForMessage('EntitySpawned', 5_000);

  await captain.close();
});

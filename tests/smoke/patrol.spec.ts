// Issue #210 — Smoke test: patrol world loads and raider entity spawns.
//
// Intercepts the default world fetch so the server loads worlds/patrol.toml
// instead of the production default world. After game start the raider entity
// (tags: ["ship","npc","enemy"]) must appear in the WorldSetup entity list,
// confirming that the ship_harrow_patrol.toml → patrol.toml → entity-spawn
// pipeline is wired end-to-end.

import { test, expect, readHostPeerId, createTestClient, waitForWasmReady, stripHeavyEntities } from './fixtures';
import fs from 'fs';
import path from 'path';

// Read patrol.toml from the source tree at test-collection time.
const PATROL_TOML = fs.readFileSync(
  path.join(__dirname, '../../assets/worlds/patrol.toml'),
  'utf-8',
);

test('patrol scenario: raider entity appears in WorldSetup after game start', async ({ context }) => {
  // Intercept the default world fetch and serve patrol.toml instead.
  // `stripHeavyEntities` removes the asteroid_field block so the lobby
  // preload gate clears in CI without waiting for ~150 MB of asteroid GLBs.
  // The raider is preserved (smoke asserts on it below).
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: stripHeavyEntities(PATROL_TOML) }),
  );

  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  const hostId = await readHostPeerId(serverPage);

  // Two-player lobby: Helm station carries CaptainChair, so the helm player starts.
  const helm = await createTestClient(context, hostId, { name: 'Helm' });
  const tactical = await createTestClient(context, hostId, { name: 'Tac' });

  await helm.send('SelectStation', { station: 'Helm' });
  await helm.page.waitForFunction(
    (t) => (window as any).__messages?.some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t,
    ),
    helm.token,
    { timeout: 5_000 },
  );

  await tactical.send('SelectStation', { station: 'Tactical' });
  await tactical.page.waitForFunction(
    (t) => (window as any).__messages?.some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t,
    ),
    tactical.token,
    { timeout: 5_000 },
  );

  await helm.send('SetReady', { ready: true });
  await tactical.send('SetReady', { ready: true });
  await helm.waitForMessage('GameStarted', 10_000);

  // WorldSetup is broadcast once after GameStarted.
  const worldSetupMsg = await helm.waitForMessage('WorldSetup', 5_000) as any;
  const entities: any[] = worldSetupMsg?.data?.world?.entities ?? [];

  // The raider (name="raider", tags include "npc" and "enemy") must be present.
  const raider = entities.find(
    (e: any) => Array.isArray(e.tags) && e.tags.includes('npc') && e.tags.includes('enemy'),
  );

  expect(
    raider,
    `Expected a raider entity (tags: npc + enemy) in WorldSetup.world.entities. Got: ${JSON.stringify(entities.map((e: any) => ({ id: e.id, tags: e.tags })))}`,
  ).toBeDefined();

  await helm.close();
  await tactical.close();
});

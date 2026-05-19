// Issue #210 — Smoke test: patrol world loads and raider entity spawns.
//
// Intercepts the default world fetch so the server loads worlds/patrol.toml
// instead of the production default world. After game start the raider entity
// (tags: ["ship","npc","enemy"]) must appear in the WorldSetup entity list,
// confirming that the pirate_raider.toml → patrol.toml → entity-spawn
// pipeline is wired end-to-end.

import { test, expect } from './fixtures';
import { readHostPeerId, createTestClient } from './fixtures';
import fs from 'fs';
import path from 'path';

// Read patrol.toml from the source tree at test-collection time.
const PATROL_TOML = fs.readFileSync(
  path.join(__dirname, '../../assets/worlds/patrol.toml'),
  'utf-8',
);

test('patrol scenario: raider entity appears in WorldSetup after game start', async ({ context }) => {
  // Intercept the default world fetch and serve patrol.toml instead.
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: PATROL_TOML }),
  );

  const serverPage = await context.newPage();
  await serverPage.goto('/');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 15_000 });

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

  await helm.send('StartGame');
  await helm.waitForMessage('GameStarted', 5_000);

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

  // The raider should be positioned near patrol_alpha anchor [300, 0, -300].
  expect(Array.isArray(raider.position)).toBe(true);
  expect(Math.abs(raider.position[0] - 300)).toBeLessThan(1.0);
  expect(Math.abs(raider.position[2] - (-300))).toBeLessThan(1.0);

  await helm.close();
  await tactical.close();
});

// Issue #222 — Smoke test: default scenario loads and Starbase Alpha spawns.
//
// Verifies the WorldPlugin bootstrap path: map config → default_scenario →
// entity spawn pipeline. After game start the station entity declared in
// assets/worlds/default.toml ("Starbase Alpha", tag: "station") must
// appear in the WorldSetup entity list.

import { test, expect, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';

test('default scenario: Starbase Alpha appears in WorldSetup after game start', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  const hostId = await readHostPeerId(serverPage);

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

  const worldSetupMsg = await helm.waitForMessage('WorldSetup', 5_000) as any;
  const entities: any[] = worldSetupMsg?.data?.world?.entities ?? [];

  // Starbase Alpha (tag: "station") must appear from assets/worlds/default.toml spawn.
  const starbase = entities.find(
    (e: any) => Array.isArray(e.tags) && e.tags.includes('station'),
  );

  expect(
    starbase,
    `Expected a station entity (Starbase Alpha) in WorldSetup. Got: ${JSON.stringify(entities.map((e: any) => ({ id: e.id, tags: e.tags })))}`,
  ).toBeDefined();

  await helm.close();
  await tactical.close();
});

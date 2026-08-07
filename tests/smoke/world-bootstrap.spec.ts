// Issue #222 — Smoke test: the default scenario loads and its station spawns.
//
// Verifies the WorldPlugin bootstrap path: map config → default_scenario →
// entity spawn pipeline. After game start the station entity the loaded world
// declares (tag: "station") must appear in the WorldSetup entity list.
//
// The world here is NOT production `assets/worlds/default.toml`: the `context`
// fixture routes that path to `MINIMAL_DEFAULT_WORLD` (see fixtures.ts), which
// declares "Starbase Alpha". The `?scenario=` query below is just the path the
// route matches on.

import {
  test,
  expect,
  readHostPeerId,
  createTestClient,
  waitForWasmReady,
  expectFixtureWorld,
  MINIMAL_DEFAULT_WORLD,
} from './fixtures';

test('default scenario: the world\'s station appears in WorldSetup after game start', async ({ context }) => {
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

  // The `context` fixture's route must actually have served
  // MINIMAL_DEFAULT_WORLD. Production `assets/worlds/default.toml` also
  // declares a `station`-tagged entity, so a route that stopped matching would
  // leave the assertion below green against production content.
  expectFixtureWorld(worldSetupMsg, MINIMAL_DEFAULT_WORLD);

  const entities: any[] = worldSetupMsg?.data?.world?.entities ?? [];

  // The fixture world's station (tag: "station") must appear from the spawn.
  const starbase = entities.find(
    (e: any) => Array.isArray(e.tags) && e.tags.includes('station'),
  );

  expect(
    starbase,
    `Expected a station entity in WorldSetup. Got: ${JSON.stringify(entities.map((e: any) => ({ id: e.id, tags: e.tags })))}`,
  ).toBeDefined();

  await helm.close();
  await tactical.close();
});

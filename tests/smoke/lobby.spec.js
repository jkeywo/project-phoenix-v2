// Issue #134 — Smoke tests: station-based lobby protocol.
// Rewrites issues #56 + #57 for SelectStation / StationAssigned.

import { test, expect, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';

async function waitForStation(client, timeout = 5_000) {
  await client.page.waitForFunction(
    (t) => window.__messages?.some(
      (m) => m.type === 'StationAssigned' && m.data.token === t
    ),
    client.token,
    { timeout },
  );
}

test('SelectStation — claims station and both clients receive StationAssigned', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  const hostId = await readHostPeerId(serverPage);

  const clientA = await createTestClient(context, hostId, { name: 'Alpha' });
  const clientB = await createTestClient(context, hostId, { name: 'Beta' });

  // Client A claims the Helm station (2P layout has "Helm" and "Tactical")
  await clientA.send('SelectStation', { station: 'Helm' });
  await waitForStation(clientA);

  const selA = await clientA.page.evaluate(
    (t) => window.__messages.find(
      (m) => m.type === 'StationAssigned' && m.data.token === t
    ),
    clientA.token,
  );
  expect(selA.data.station).toBe('Helm');
  // Station holder: `station_id` is the lowercase station id (bare string).
  expect(selA.data.station_id).toBe('helm');

  // Client B should also receive the StationAssigned broadcast
  const selAonB = await clientB.page.evaluate(
    (t) => window.__messages.find(
      (m) => m.type === 'StationAssigned' && m.data.token === t
    ),
    clientA.token,
  );
  expect(selAonB.data.station).toBe('Helm');

  await clientA.close();
  await clientB.close();
});

test('all players SetReady starts the game', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  const hostId = await readHostPeerId(serverPage);

  const clientA = await createTestClient(context, hostId, { name: 'Helm' });
  const clientB = await createTestClient(context, hostId, { name: 'Tactical' });

  await clientA.send('SelectStation', { station: 'Helm' });
  await waitForStation(clientA);

  await clientB.send('SelectStation', { station: 'Tactical' });
  await waitForStation(clientB);

  // Game starts when ALL connected players are ready (auto-start).
  await clientA.send('SetReady', { ready: true });
  await clientB.send('SetReady', { ready: true });

  await clientA.waitForMessage('GameStarted', 10_000);
  await clientB.waitForMessage('GameStarted', 10_000);

  await clientA.close();
  await clientB.close();
});

test('SetReady starts game even with unfilled stations', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  const hostId = await readHostPeerId(serverPage);

  const clientA = await createTestClient(context, hostId, { name: 'Helm' });
  const clientB = await createTestClient(context, hostId, { name: 'Spectator' });

  // Only A claims Helm — Tactical station is unfilled
  await clientA.send('SelectStation', { station: 'Helm' });
  await waitForStation(clientA);

  // Both connected players must be ready (auto-start). Unfilled stations are OK.
  await clientA.send('SetReady', { ready: true });
  await clientB.send('SetReady', { ready: true });

  await clientA.waitForMessage('GameStarted', 10_000);
  await clientB.waitForMessage('GameStarted', 10_000);

  await clientA.close();
  await clientB.close();
});

test('SetReady from all players starts game and Welcome has Lobby phase', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  const hostId = await readHostPeerId(serverPage);

  const clientA = await createTestClient(context, hostId, { name: 'Helm' });
  const clientB = await createTestClient(context, hostId, { name: 'Tactical' });

  await clientA.send('SelectStation', { station: 'Helm' });
  await waitForStation(clientA);

  await clientB.send('SelectStation', { station: 'Tactical' });
  await waitForStation(clientB);

  // Both players ready -> auto-start
  await clientA.send('SetReady', { ready: true });
  await clientB.send('SetReady', { ready: true });

  await clientA.waitForMessage('GameStarted', 10_000);
  await clientB.waitForMessage('GameStarted', 10_000);

  // Welcome was sent with Lobby phase before game started
  const welcomeA = await clientA.page.evaluate(
    () => window.__messages.find((m) => m.type === 'Welcome'),
  );
  expect(welcomeA.data.state.phase).toBe('Lobby');

  await clientA.close();
  await clientB.close();
});
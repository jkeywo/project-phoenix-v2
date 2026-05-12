// Issue #134 — Smoke tests: station-based lobby protocol.
// Rewrites issues #56 + #57 for SelectStation / StationAssigned.

import { test, expect } from './fixtures';
import { readHostPeerId, createTestClient } from './fixtures';

async function waitForStation(client: { page: import('@playwright/test').Page; token: string }, timeout = 5_000) {
  await client.page.waitForFunction(
    (t) => (window as any).__messages?.some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t
    ),
    client.token,
    { timeout },
  );
}

test('SelectStation — claims station and both clients receive StationAssigned', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 15_000 });

  const hostId = await readHostPeerId(serverPage);

  const clientA = await createTestClient(context, hostId, { name: 'Alpha' });
  const clientB = await createTestClient(context, hostId, { name: 'Beta' });

  // Client A claims the Helm station (2P layout has "Helm" and "Tactical")
  await clientA.send('SelectStation', { station: 'Helm' });
  await waitForStation(clientA);

  const selA = await clientA.page.evaluate(
    (t) => (window as any).__messages.find(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t
    ),
    clientA.token,
  ) as any;
  expect(selA.data.station).toBe('Helm');
  expect(Array.isArray(selA.data.consoles)).toBe(true);
  expect(selA.data.consoles.length).toBeGreaterThan(0);

  // Client B should also receive the StationAssigned broadcast
  const selAonB = await clientB.page.evaluate(
    (t) => (window as any).__messages.find(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t
    ),
    clientA.token,
  ) as any;
  expect(selAonB.data.station).toBe('Helm');

  await clientA.close();
  await clientB.close();
});

test('non-captain StartGame is ignored', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 15_000 });

  const hostId = await readHostPeerId(serverPage);

  // Two clients: A takes Helm (CaptainChair), B takes Tactical
  const clientA = await createTestClient(context, hostId, { name: 'Helm' });
  const clientB = await createTestClient(context, hostId, { name: 'Tactical' });

  await clientA.send('SelectStation', { station: 'Helm' });
  await waitForStation(clientA);

  await clientB.send('SelectStation', { station: 'Tactical' });
  await waitForStation(clientB);

  // Non-captain (B / Tactical station) attempts StartGame — should be ignored
  await clientB.send('StartGame');

  await clientA.page.waitForTimeout(500);
  const earlyA = await clientA.lastMessage('GameStarted');
  expect(earlyA).toBeNull();

  await clientA.close();
  await clientB.close();
});

test('StartGame with unfilled stations is ignored', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 15_000 });

  const hostId = await readHostPeerId(serverPage);

  // Two clients join; only one claims a station
  const clientA = await createTestClient(context, hostId, { name: 'Helm' });
  const clientB = await createTestClient(context, hostId, { name: 'Spectator' });

  // Only A claims Helm — Tactical station is unfilled
  await clientA.send('SelectStation', { station: 'Helm' });
  await waitForStation(clientA);

  // A is the captain (holds CaptainChair via Helm station) but stations are not all filled
  await clientA.send('StartGame');

  await clientA.page.waitForTimeout(500);
  const earlyA = await clientA.lastMessage('GameStarted');
  expect(earlyA).toBeNull();

  await clientA.close();
  await clientB.close();
});

test('captain starts game — both clients receive GameStarted', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 15_000 });

  const hostId = await readHostPeerId(serverPage);

  const clientA = await createTestClient(context, hostId, { name: 'Helm' });
  const clientB = await createTestClient(context, hostId, { name: 'Tactical' });

  // 2P layout: Helm (CaptainChair+Helm) + Tactical (Tactical+Engineering)
  await clientA.send('SelectStation', { station: 'Helm' });
  await waitForStation(clientA);

  await clientB.send('SelectStation', { station: 'Tactical' });
  await waitForStation(clientB);

  // Captain (A, holds CaptainChair via Helm station) sends StartGame
  await clientA.send('StartGame');

  await clientA.waitForMessage('GameStarted', 5_000);
  await clientB.waitForMessage('GameStarted', 5_000);

  // Welcome was sent with Lobby phase before game started
  const welcomeA = await clientA.page.evaluate(
    () => (window as any).__messages.find((m: any) => m.type === 'Welcome'),
  ) as any;
  expect(welcomeA.data.state.phase).toBe('Lobby');

  await clientA.close();
  await clientB.close();
});
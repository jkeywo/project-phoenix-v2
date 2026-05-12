// Issue #134 — Smoke tests: multi-client station reassignment cascades.
// Covers: leave cascade, spectator promotion on max-players leave.

import { test, expect } from './fixtures';
import { readHostPeerId, createTestClient } from './fixtures';
import type { TestClient } from './fixtures';
import type { BrowserContext } from '@playwright/test';

/** Boot a fresh server page and return the host peer ID. */
async function bootServer(context: BrowserContext): Promise<string> {
  const serverPage = await context.newPage();
  await serverPage.goto('/');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 15_000 });
  return readHostPeerId(serverPage);
}

/** Return the last StationAssigned message received for a given token. */
async function lastAssignment(client: TestClient, token: string) {
  return client.page.evaluate(
    (t) => {
      const msgs: any[] = (window as any).__messages || [];
      const all = msgs.filter((m: any) => m.type === 'StationAssigned' && m.data.token === t);
      return all.length > 0 ? all[all.length - 1] : null;
    },
    token,
  ) as Promise<any>;
}

test('3 players can each claim a station at 3P layout', async ({ context }) => {
  const hostId = await bootServer(context);

  const c1 = await createTestClient(context, hostId, { name: 'P1' });
  const c2 = await createTestClient(context, hostId, { name: 'P2' });
  const c3 = await createTestClient(context, hostId, { name: 'P3' });

  // At 3P layout: Helm, Tactical, Engineering
  await c1.send('SelectStation', { station: 'Helm' });
  await c1.waitForMessage('StationAssigned', 5_000);

  await c2.send('SelectStation', { station: 'Tactical' });
  await c2.waitForMessage('StationAssigned', 5_000);

  await c3.send('SelectStation', { station: 'Engineering' });
  await c3.waitForMessage('StationAssigned', 5_000);

  const a1 = await lastAssignment(c1, c1.token) as any;
  const a2 = await lastAssignment(c2, c2.token) as any;
  const a3 = await lastAssignment(c3, c3.token) as any;

  expect(a1.data.station).toBe('Helm');
  expect(a1.data.consoles).toContain('CaptainChair');
  expect(a2.data.station).toBe('Tactical');
  expect(a3.data.station).toBe('Engineering');
  expect(a3.data.consoles).toContain('Engineering');

  await c1.close();
  await c2.close();
  await c3.close();
});

test('3→2 player leave: remaining players keep their stations', async ({ context }) => {
  const hostId = await bootServer(context);

  const c1 = await createTestClient(context, hostId, { name: 'P1' });
  const c2 = await createTestClient(context, hostId, { name: 'P2' });

  await c1.send('SelectStation', { station: 'Helm' });
  await c1.waitForMessage('StationAssigned', 5_000);

  await c2.send('SelectStation', { station: 'Tactical' });
  await c2.waitForMessage('StationAssigned', 5_000);

  const c3 = await createTestClient(context, hostId, { name: 'P3' });
  await c3.send('SelectStation', { station: 'Engineering' });
  await c3.waitForMessage('StationAssigned', 5_000);

  // c3 disconnects — 3P→2P leave cascade
  await c3.close();

  // Wait for the cascade to settle
  await c1.page.waitForTimeout(500);

  // c1 and c2 should still be on their 2P stations (Helm persists, Tactical persists)
  const a1 = await lastAssignment(c1, c1.token) as any;
  const a2 = await lastAssignment(c2, c2.token) as any;

  expect(a1.data.station).toBe('Helm');
  expect(a2.data.station).toBe('Tactical');
});

test('leave at max_players allows spectator to claim vacated station', async ({ context }) => {
  const hostId = await bootServer(context);

  // Fill all 3 stations (max_players = 3)
  const c1 = await createTestClient(context, hostId, { name: 'P1' });
  const c2 = await createTestClient(context, hostId, { name: 'P2' });
  const c3 = await createTestClient(context, hostId, { name: 'P3' });

  await c1.send('SelectStation', { station: 'Helm' });
  await c1.waitForMessage('StationAssigned', 5_000);

  await c2.send('SelectStation', { station: 'Tactical' });
  await c2.waitForMessage('StationAssigned', 5_000);

  await c3.send('SelectStation', { station: 'Engineering' });
  await c3.waitForMessage('StationAssigned', 5_000);

  // 4th player joins as spectator (at max_players)
  const c4 = await createTestClient(context, hostId, { name: 'Spectator' });

  // Wait for spectator StationAssigned (station: null)
  await c4.page.waitForFunction(
    (token) => ((window as any).__messages as any[]).some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === token && m.data.station === null
    ),
    c4.token,
    { timeout: 5_000 },
  );

  // c3 disconnects (held Engineering) — c4 should now be able to claim it
  await c3.close();

  // Wait for cascade to settle
  await c4.page.waitForTimeout(500);

  // c4 claims Engineering (now vacant at the current 3-connected-player count)
  await c4.send('SelectStation', { station: 'Engineering' });

  await c4.page.waitForFunction(
    (token) => ((window as any).__messages as any[]).some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === token && m.data.station !== null
    ),
    c4.token,
    { timeout: 5_000 },
  );

  const a4 = await lastAssignment(c4, c4.token) as any;
  expect(a4.data.station).not.toBeNull();
  expect(a4.data.consoles.length).toBeGreaterThan(0);

  await c1.close();
  await c2.close();
  await c4.close();
});

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

/**
 * Send SelectStation and wait for a StationAssigned *for this client's token*
 * so we don't accidentally catch advance broadcasts for other players.
 */
async function selectAndWait(client: TestClient, station: string, timeout = 5_000) {
  await client.send('SelectStation', { station });
  await client.page.waitForFunction(
    (t) => (window as any).__messages?.some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t
    ),
    client.token,
    { timeout },
  );
}

test('3 players can each claim a station at 3P layout', async ({ context }) => {
  const hostId = await bootServer(context);

  // Build up 1P→2P→3P one player at a time, selecting between joins,
  // so advance_on_join works naturally and there are no stale messages.
  const c1 = await createTestClient(context, hostId, { name: 'P1' });

  // At 1P only "Captain" exists. Select it — advance_on_join will move c1 to
  // Helm at 2P/3P when the others join.
  await selectAndWait(c1, 'Captain');

  const c2 = await createTestClient(context, hostId, { name: 'P2' });
  // advance_on_join already moved c1→Helm at 2P. c2 selects Tactical.
  await selectAndWait(c2, 'Tactical');

  const c3 = await createTestClient(context, hostId, { name: 'P3' });
  // advance_on_join moved c1→Helm, c2→Tactical at 3P. c3 selects Repair.

  await selectAndWait(c3, 'Repair');

  const a1 = await lastAssignment(c1, c1.token) as any;
  const a2 = await lastAssignment(c2, c2.token) as any;
  const a3 = await lastAssignment(c3, c3.token) as any;

  expect(a1.data.station).toBe('Helm');
  expect(a1.data.consoles).toContain('CaptainChair');
  expect(a2.data.station).toBe('Tactical');
  expect(a3.data.station).toBe('Repair');
  expect(a3.data.consoles).toContain('Repair');

  await c1.close();
  await c2.close();
  await c3.close();
});

test('3→2 player leave: remaining players keep their stations', async ({ context }) => {
  const hostId = await bootServer(context);

  const c1 = await createTestClient(context, hostId, { name: 'P1' });
  await selectAndWait(c1, 'Captain');

  const c2 = await createTestClient(context, hostId, { name: 'P2' });
  await selectAndWait(c2, 'Tactical');

  const c3 = await createTestClient(context, hostId, { name: 'P3' });
  await selectAndWait(c3, 'Repair');

  // c3 disconnects — 3P→2P leave cascade
  await c3.close();

  // Wait for the cascade to settle
  await c1.page.waitForTimeout(500);

  // c1 and c2 should still be on their 2P stations (Helm persists, Tactical persists)
  const a1 = await lastAssignment(c1, c1.token) as any;
  const a2 = await lastAssignment(c2, c2.token) as any;

  expect(a1.data.station).toBe('Helm');
  expect(a2.data.station).toBe('Tactical');

  await c1.close();
  await c2.close();
});

test('leave at max_players allows spectator to claim vacated station', async ({ context }) => {
  const hostId = await bootServer(context);

  // Fill all 3 stations (max_players = 3)
  const c1 = await createTestClient(context, hostId, { name: 'P1' });
  await selectAndWait(c1, 'Captain');

  const c2 = await createTestClient(context, hostId, { name: 'P2' });
  await selectAndWait(c2, 'Tactical');

  const c3 = await createTestClient(context, hostId, { name: 'P3' });
  await selectAndWait(c3, 'Repair');

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

  // c3 disconnects (held Repair) → consoles cleared. c4 can claim it.
  await c3.close();

  // Wait for cascade to settle
  await c4.page.waitForTimeout(500);

  // c4 claims Repair (vacated by c3's disconnect)
  await c4.send('SelectStation', { station: 'Repair' });
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
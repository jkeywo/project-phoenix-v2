// Issue #495 — Smoke tests: fixed-roster station assignments.
// With the fixed-roster model (always 6P layout), there is no cascade on
// join or leave. Players keep their selected station; leavers' stations
// become free for others to claim manually via SelectStation.

import { test, expect } from './fixtures';
import { readHostPeerId, createTestClient, createServerPage } from './fixtures';
import type { TestClient } from './fixtures';
import type { BrowserContext } from '@playwright/test';

/** Boot a fresh server page and return the host peer ID. */
async function bootServer(context: BrowserContext): Promise<string> {
  const serverPage = await createServerPage(context);
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
 * so we don't accidentally catch broadcasts for other players.
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

test('3 players each claim a station at the fixed 6P layout', async ({ context }) => {
  const hostId = await bootServer(context);

  const c1 = await createTestClient(context, hostId, { name: 'P1' });
  await selectAndWait(c1, 'Captain');

  const c2 = await createTestClient(context, hostId, { name: 'P2' });
  await selectAndWait(c2, 'Helm');

  const c3 = await createTestClient(context, hostId, { name: 'P3' });
  await selectAndWait(c3, 'Tactical');

  const a1 = await lastAssignment(c1, c1.token) as any;
  const a2 = await lastAssignment(c2, c2.token) as any;
  const a3 = await lastAssignment(c3, c3.token) as any;

  expect(a1.data.station).toBe('Captain');
  expect(a1.data.station_id).toBe('captain');

  expect(a2.data.station).toBe('Helm');
  expect(a2.data.station_id).toBe('helm');

  expect(a3.data.station).toBe('Tactical');
  expect(a3.data.station_id).toBe('tactical');

  await c1.close();
  await c2.close();
  await c3.close();
});

test('leave does not change remaining players stations (fixed roster)', async ({ context }) => {
  const hostId = await bootServer(context);

  const c1 = await createTestClient(context, hostId, { name: 'P1' });
  await selectAndWait(c1, 'Captain');

  const c2 = await createTestClient(context, hostId, { name: 'P2' });
  await selectAndWait(c2, 'Helm');

  const c3 = await createTestClient(context, hostId, { name: 'P3' });
  await selectAndWait(c3, 'Tactical');

  // c3 disconnects — fixed roster: no cascade, c1/c2 keep their stations.
  await c3.close();

  // Give server time to process the disconnect
  await c1.page.waitForTimeout(500);

  const a1 = await lastAssignment(c1, c1.token) as any;
  const a2 = await lastAssignment(c2, c2.token) as any;

  expect(a1.data.station).toBe('Captain');
  expect(a2.data.station).toBe('Helm');

  await c1.close();
  await c2.close();
});

test('leaver station can be claimed by another player', async ({ context }) => {
  const hostId = await bootServer(context);

  const c1 = await createTestClient(context, hostId, { name: 'P1' });
  await selectAndWait(c1, 'Captain');

  const c2 = await createTestClient(context, hostId, { name: 'P2' });
  await selectAndWait(c2, 'Helm');

  // c1 disconnects — Captain station becomes free.
  await c1.close();
  await c2.page.waitForTimeout(500);

  // c3 claims the vacated Captain station.
  const c3 = await createTestClient(context, hostId, { name: 'P3' });
  await selectAndWait(c3, 'Captain');

  const a3 = await lastAssignment(c3, c3.token) as any;
  expect(a3.data.station).toBe('Captain');
  expect(a3.data.station_id).toBe('captain');

  await c2.close();
  await c3.close();
});

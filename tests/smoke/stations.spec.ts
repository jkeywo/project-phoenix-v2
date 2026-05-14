// Issue #134 — Smoke tests: station contract (Welcome, spectator overflow,
// SelectStation / ReleaseStation behaviors).

import { test, expect } from './fixtures';
import { readHostPeerId, createTestClient, createServerPage } from './fixtures';

test('Welcome includes ship_stations', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 15_000 });

  const hostId = await readHostPeerId(serverPage);
  const client = await createTestClient(context, hostId, { name: 'Tester' });

  const welcome = await client.page.evaluate(
    () => (window as any).__messages.find((m: any) => m.type === 'Welcome'),
  ) as any;

  expect(welcome).not.toBeNull();
  expect(welcome.data).toHaveProperty('ship_stations');
  const ss = welcome.data.ship_stations;
  // Should have configs for multiple player counts
  expect(typeof ss).toBe('object');
  // min_players and max_players are present
  expect(typeof ss.min_players).toBe('number');
  expect(typeof ss.max_players).toBe('number');

  await client.close();
});

test('SelectStation for empty station claims it and broadcasts StationAssigned', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 15_000 });

  const hostId = await readHostPeerId(serverPage);
  const client = await createTestClient(context, hostId, { name: 'Solo' });

  await client.send('SelectStation', { station: 'Captain' });

  const msg = await client.waitForMessage('StationAssigned', 5_000) as any;
  expect(msg.data.token).toBe(client.token);
  expect(msg.data.station).toBe('Captain');
  expect(msg.data.consoles).toContain('CaptainChair');

  await client.close();
});

test('SelectStation for occupied station is a no-op', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 15_000 });

  const hostId = await readHostPeerId(serverPage);

  const clientA = await createTestClient(context, hostId, { name: 'A' });
  const clientB = await createTestClient(context, hostId, { name: 'B' });

  // A claims Helm at 2P layout
  await clientA.send('SelectStation', { station: 'Helm' });
  await clientA.waitForMessage('StationAssigned', 5_000);

  // Clear B's messages
  await clientB.page.evaluate(() => { (window as any).__messages = []; });

  // B tries to claim the same station — should be no-op
  await clientB.send('SelectStation', { station: 'Helm' });

  // Wait briefly; B should not receive any StationAssigned for themselves
  await clientB.page.waitForTimeout(500);
  const msgs = await clientB.page.evaluate(
    () => ((window as any).__messages as any[]).filter((m: any) => m.type === 'StationAssigned' && m.data.token !== 'tc-ignored')
  ) as any[];
  // B should get no StationAssigned (the occupied station claim is dropped)
  const assignedToB = msgs.filter((m: any) => m.data.token === clientB.token);
  expect(assignedToB.length).toBe(0);

  await clientA.close();
  await clientB.close();
});

test('SelectStation for own station is a no-op', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 15_000 });

  const hostId = await readHostPeerId(serverPage);
  const client = await createTestClient(context, hostId, { name: 'Solo' });

  await client.send('SelectStation', { station: 'Captain' });
  await client.waitForMessage('StationAssigned', 5_000);

  // Clear messages and select own station again
  await client.page.evaluate(() => { (window as any).__messages = []; });

  await client.send('SelectStation', { station: 'Captain' });

  await client.page.waitForTimeout(500);
  const msgs = await client.page.evaluate(
    () => (window as any).__messages as any[],
  ) as any[];
  expect(msgs.filter((m: any) => m.type === 'StationAssigned').length).toBe(0);

  await client.close();
});

test('ReleaseStation returns player to spectator', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 15_000 });

  const hostId = await readHostPeerId(serverPage);
  const client = await createTestClient(context, hostId, { name: 'Solo' });

  await client.send('SelectStation', { station: 'Captain' });
  await client.waitForMessage('StationAssigned', 5_000);

  // Clear and release
  await client.page.evaluate(() => { (window as any).__messages = []; });
  await client.send('ReleaseStation');

  const msg = await client.waitForMessage('StationAssigned', 5_000) as any;
  expect(msg.data.token).toBe(client.token);
  expect(msg.data.station).toBeNull();
  expect(msg.data.consoles).toHaveLength(0);

  await client.close();
});

test('first connector when full becomes spectator', async ({ context }) => {
  const serverPage = await createServerPage(context, { maxPlayers: 3 });
  const hostId = await readHostPeerId(serverPage);

  // At 3P max, fill all 3 stations
  const c1 = await createTestClient(context, hostId, { name: 'P1' });
  const c2 = await createTestClient(context, hostId, { name: 'P2' });
  const c3 = await createTestClient(context, hostId, { name: 'P3' });

  await c1.send('SelectStation', { station: 'Helm' });
  await c1.waitForMessage('StationAssigned', 5_000);

  await c2.send('SelectStation', { station: 'Tactical' });
  await c2.waitForMessage('StationAssigned', 5_000);

  await c3.send('SelectStation', { station: 'Engineering' });
  await c3.waitForMessage('StationAssigned', 5_000);

  // 4th player joins — at max_players (3), should be spectator
  const c4 = await createTestClient(context, hostId, { name: 'Spectator' });

  // The Welcome or auto-assignment should mark c4 as spectator (station=null)
  const spectatorMsg = await c4.page.evaluate(
    () => {
      const msgs: any[] = (window as any).__messages || [];
      return msgs.find((m: any) => m.type === 'StationAssigned' && m.data.token === (window as any).__myToken);
    }
  ) as any;

  // If not found by evaluating __myToken trick, check the StationAssigned for c4's token
  const spectatorMsg2 = await c4.page.evaluate(
    (token) => {
      const msgs: any[] = (window as any).__messages || [];
      return msgs.find((m: any) => m.type === 'StationAssigned' && m.data.token === token);
    },
    c4.token,
  ) as any;

  expect(spectatorMsg2).not.toBeNull();
  expect(spectatorMsg2.data.station).toBeNull();
  expect(spectatorMsg2.data.consoles).toHaveLength(0);

  await c1.close();
  await c2.close();
  await c3.close();
  await c4.close();
});

// Issue #134 — Smoke tests: station contract (Welcome, spectator overflow,
// SelectStation / ReleaseStation behaviors).

import { test, expect, readHostPeerId, createTestClient, createServerPage, waitForWasmReady } from './fixtures';

test('Welcome includes ship_stations', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  const hostId = await readHostPeerId(serverPage);
  const client = await createTestClient(context, hostId, { name: 'Tester' });

  const welcome = await client.page.evaluate(
    () => (window as any).__messages.find((m: any) => m.type === 'Welcome'),
  ) as any;

  expect(welcome).not.toBeNull();
  expect(welcome.data).toHaveProperty('ship_stations');
  const ss = welcome.data.ship_stations;
  // Flat station roster — should be an object with a `stations` array
  expect(typeof ss).toBe('object');
  expect(Array.isArray(ss.stations)).toBe(true);
  expect(ss.stations.length).toBeGreaterThan(0);
  // Each station has an id, name, and rank (flat StationDef post-#619).
  const first = ss.stations[0];
  expect(typeof first.id).toBe('string');
  expect(typeof first.name).toBe('string');
  expect(typeof first.rank).toBe('string');

  await client.close();
});

test('SelectStation for empty station claims it and broadcasts StationAssigned', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  const hostId = await readHostPeerId(serverPage);
  const client = await createTestClient(context, hostId, { name: 'Solo' });

  await client.send('SelectStation', { station: 'Captain' });

  const msg = await client.waitForMessage('StationAssigned', 5_000) as any;
  expect(msg.data.token).toBe(client.token);
  expect(msg.data.station).toBe('Captain');
  expect(msg.data.station_id).toBe('captain');

  await client.close();
});

test('SelectStation for occupied station is a no-op', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

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
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

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
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

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
  // Spectator: serde omits `station_id` when None, so the field is undefined.
  expect(msg.data.station_id).toBeUndefined();

  await client.close();
});

test('10th connector when all 9 stations are filled becomes spectator', async ({ context }) => {
  const serverPage = await createServerPage(context);
  const hostId = await readHostPeerId(serverPage);

  const stations = ['Captain', 'Helm', 'Tactical', 'Repair', 'Sensors', 'Shields', 'Navigation', 'Power', 'Comms'];
  const clients = [];
  for (let i = 0; i < stations.length; i++) {
    const c = await createTestClient(context, hostId, { name: `P${i + 1}` });
    await c.send('SelectStation', { station: stations[i] });
    await c.waitForMessage('StationAssigned', 5_000);
    clients.push(c);
  }

  // 10th player joins — all 9 stations filled, should be spectator
  const c10 = await createTestClient(context, hostId, { name: 'Spectator' });

  const spectatorMsg = await c10.page.evaluate(
    () => {
      const msgs: any[] = (window as any).__messages || [];
      return msgs.find((m: any) => m.type === 'StationAssigned' && m.data.token === (window as any).__myToken);
    }
  ) as any;

  const spectatorMsg2 = await c10.page.evaluate(
    (token) => {
      const msgs: any[] = (window as any).__messages || [];
      return msgs.find((m: any) => m.type === 'StationAssigned' && m.data.token === token);
    },
    c10.token,
  ) as any;

  expect(spectatorMsg2).not.toBeNull();
  expect(spectatorMsg2.data.station).toBeNull();
  // Spectator: serde omits `station_id` when None, so the field is undefined.
  expect(spectatorMsg2.data.station_id).toBeUndefined();

  for (const c of clients) { await c.close(); }
  await c10.close();
});

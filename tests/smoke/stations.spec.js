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
    () => window.__messages.find((m) => m.type === 'Welcome'),
  );

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

  const msg = await client.waitForMessage('StationAssigned', 5_000);
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
  await clientB.page.evaluate(() => { window.__messages = []; });

  // B tries to claim the same station — should be no-op
  await clientB.send('SelectStation', { station: 'Helm' });

  // Wait briefly; B should not receive any StationAssigned for themselves
  await clientB.page.waitForTimeout(500);
  const msgs = await clientB.page.evaluate(
    () => window.__messages.filter((m) => m.type === 'StationAssigned' && m.data.token !== 'tc-ignored')
  );
  // B should get no StationAssigned (the occupied station claim is dropped)
  const assignedToB = msgs.filter((m) => m.data.token === clientB.token);
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
  await client.page.evaluate(() => { window.__messages = []; });

  await client.send('SelectStation', { station: 'Captain' });

  await client.page.waitForTimeout(500);
  const msgs = await client.page.evaluate(
    () => window.__messages,
  );
  expect(msgs.filter((m) => m.type === 'StationAssigned').length).toBe(0);

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
  await client.page.evaluate(() => { window.__messages = []; });
  await client.send('ReleaseStation');

  const msg = await client.waitForMessage('StationAssigned', 5_000);
  expect(msg.data.token).toBe(client.token);
  expect(msg.data.station).toBeNull();
  // Spectator: serde omits `station_id` when None, so the field is undefined.
  expect(msg.data.station_id).toBeUndefined();

  await client.close();
});

// Issue #941: this used to hardcode `['Captain', 'Helm', 'Tactical', 'Science',
// 'Engineering', 'Comms']` and call itself "7th connector when all 6 stations
// are filled" — the roster the player hull happened to author. A hull gaining
// or losing a station broke it, and worse, a hull losing one would have made it
// pass vacuously (the "overflow" player would just have taken the free seat).
// The roster now comes off the wire from `Welcome.ship_stations`, which is the
// same list the real lobby renders, so the test means "one more player than the
// ship has seats becomes a spectator" for whatever ship is loaded.
test('a connector arriving after every station is filled becomes a spectator', async ({ context }) => {
  const serverPage = await createServerPage(context);
  const hostId = await readHostPeerId(serverPage);

  const first = await createTestClient(context, hostId, { name: 'P1' });
  const welcome = await first.page.evaluate(
    () => window.__messages.find((m) => m.type === 'Welcome'),
  );
  const roster = welcome.data.ship_stations.stations;
  expect(roster.length, 'the ship must declare at least one station to fill').toBeGreaterThan(0);

  // `SelectStation` accepts either the station's display name or its id
  // (`lobby::stations_config::get_station`); the id is the stabler key.
  const clients = [first];
  for (const [i, station] of roster.entries()) {
    const c = i === 0 ? first : await createTestClient(context, hostId, { name: `P${i + 1}` });
    if (i > 0) clients.push(c);
    await c.send('SelectStation', { station: station.id });
    await c.waitForMessage('StationAssigned', 5_000);
  }

  // Every seat is now held, so the next player to connect has nowhere to sit.
  const overflow = await createTestClient(context, hostId, { name: 'Spectator' });

  const spectatorMsg = await overflow.page.evaluate(
    (token) => {
      const msgs = window.__messages || [];
      return msgs.find((m) => m.type === 'StationAssigned' && m.data.token === token);
    },
    overflow.token,
  );

  expect(spectatorMsg).not.toBeNull();
  expect(spectatorMsg.data.station).toBeNull();
  // Spectator: serde omits `station_id` when None, so the field is undefined.
  expect(spectatorMsg.data.station_id).toBeUndefined();

  for (const c of clients) { await c.close(); }
  await overflow.close();
});

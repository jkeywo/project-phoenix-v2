// Smoke tests: browser-refresh auto-rejoin. A client that drops and reconnects
// with the same session token must land back on the station it held before —
// in the lobby and (the headline case) mid-game. Exercises the reserve-the-seat
// disconnect, the all-phases Identify handshake, and the server.html peer remap.

import { test, expect } from './fixtures';
import { readHostPeerId, createTestClient, createServerPage } from './fixtures';

async function bootServer(context) {
  const serverPage = await createServerPage(context);
  return readHostPeerId(serverPage);
}

/** Pull the station name the given token holds out of a Welcome message payload. */
function stationInWelcome(welcome, token) {
  const players = welcome?.data?.state?.players ?? [];
  const me = players.find((p) => p.token === token);
  return me?.station ?? null;
}

/** Send SelectStation and wait for this token's StationAssigned to land. */
async function selectAndWait(client, station) {
  await client.send('SelectStation', { station });
  await client.page.waitForFunction(
    (t) => window.__messages?.some(
      (m) => m.type === 'StationAssigned' && m.data.token === t && m.data.station !== null,
    ),
    client.token,
    { timeout: 5_000 },
  );
}

test('refresh in the lobby rejoins the same station', async ({ context }) => {
  const hostId = await bootServer(context);
  const TOKEN = 'reconnect-lobby';

  const c1 = await createTestClient(context, hostId, { token: TOKEN, name: 'P1' });
  await selectAndWait(c1, 'Captain');

  // Simulate a browser refresh: drop the connection, then reconnect with the
  // same token (createTestClient waits for the fresh Welcome).
  await c1.close();
  const c1b = await createTestClient(context, hostId, { token: TOKEN, name: 'P1' });

  const welcome = await c1b.waitForMessage('Welcome');
  const station = stationInWelcome(welcome, TOKEN);
  expect(station).toBe('captain');

  await c1b.close();
});

test('refresh mid-game rejoins straight onto the same console', { tag: '@core' }, async ({ context }) => {
  const hostId = await bootServer(context);
  const TOKEN = 'reconnect-ingame';

  // Solo crew: at 1P the "Captain" station covers every console, so one player
  // fills all stations and can Engage.
  const c1 = await createTestClient(context, hostId, { token: TOKEN, name: 'P1' });
  await selectAndWait(c1, 'Captain');
  await c1.send('SetReady', { ready: true });
  await c1.waitForMessage('GameStarted');

  // Refresh mid-game.
  await c1.close();
  const c1b = await createTestClient(context, hostId, { token: TOKEN, name: 'P1' });

  const welcome = await c1b.waitForMessage('Welcome');
  expect(welcome?.data?.state?.phase).toBe('InProgress');
  const station = stationInWelcome(welcome, TOKEN);
  expect(station).toBe('captain');

  await c1b.close();
});

test('replacement retains its Station when the old page closes after the handoff', async ({ context }) => {
  const hostId = await bootServer(context);
  const token = 'overlapping-reconnect';
  const old = await createTestClient(context, hostId, { token, name: 'P1' });
  await selectAndWait(old, 'Captain');

  const current = await createTestClient(context, hostId, { token, name: 'P1' });
  expect(stationInWelcome(await current.waitForMessage('Welcome'), token)).toBe('captain');
  // Old page teardown comes AFTER the new Identify/Welcome, not before it.
  await old.close();
  await current.send('SetReady', { ready: true });
  await current.waitForMessage('GameStarted');
  await current.send('Identify', { token, name: 'P1' });
  await current.page.waitForFunction(t => window.__messages.some(m =>
    m.type === 'Welcome' && m.data.state.phase === 'InProgress' &&
    m.data.state.players.some(p => p.token === t && p.station === 'captain')
  ), token);
  await current.close();
});

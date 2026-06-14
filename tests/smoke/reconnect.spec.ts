// Smoke tests: browser-refresh auto-rejoin. A client that drops and reconnects
// with the same session token must land back on the station it held before —
// in the lobby and (the headline case) mid-game. Exercises the reserve-the-seat
// disconnect, the all-phases Identify handshake, and the server.html peer remap.

import { test, expect } from './fixtures';
import { readHostPeerId, createTestClient, createServerPage } from './fixtures';
import type { TestClient } from './fixtures';
import type { BrowserContext } from '@playwright/test';

async function bootServer(context: BrowserContext): Promise<string> {
  const serverPage = await createServerPage(context);
  return readHostPeerId(serverPage);
}

/** Pull the consoles the given token holds out of a Welcome message payload. */
function consolesInWelcome(welcome: any, token: string): string[] {
  const players = welcome?.data?.state?.players ?? [];
  const me = players.find((p: any) => p.token === token);
  return (me && (me.consoles ?? (me.console ? [me.console] : []))) || [];
}

/** Send SelectStation and wait for this token's StationAssigned to land. */
async function selectAndWait(client: TestClient, station: string) {
  await client.send('SelectStation', { station });
  await client.page.waitForFunction(
    (t) => (window as any).__messages?.some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t && m.data.station !== null,
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
  const consoles = consolesInWelcome(welcome, TOKEN);
  expect(consoles).toContain('CaptainChair');

  await c1b.close();
});

test('refresh mid-game rejoins straight onto the same console', async ({ context }) => {
  const hostId = await bootServer(context);
  const TOKEN = 'reconnect-ingame';

  // Solo crew: at 1P the "Captain" station covers every console, so one player
  // fills all stations and can Engage.
  const c1 = await createTestClient(context, hostId, { token: TOKEN, name: 'P1' });
  await selectAndWait(c1, 'Captain');
  await c1.send('StartGame');
  await c1.waitForMessage('GameStarted');

  // Refresh mid-game.
  await c1.close();
  const c1b = await createTestClient(context, hostId, { token: TOKEN, name: 'P1' });

  const welcome = await c1b.waitForMessage('Welcome');
  expect(welcome?.data?.state?.phase).toBe('InProgress');
  const consoles = consolesInWelcome(welcome, TOKEN);
  expect(consoles).toContain('CaptainChair');

  await c1b.close();
});

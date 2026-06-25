// Issue #306 — Smoke test: SetView produces correct SimSnapshot view_mode.
//
// 9P fixed-roster layout (max_players=9, but we use 5 where only 4 are needed):
//   Captain station  → CaptainChair
//   Helm station     → Helm
//   Comms station    → Comms
//   Sensors station  → Sensors
//   Navigation station → Navigation
//
// Authorisation rules under test:
//   Captain (CaptainChair) → Camera(Fore/Aft/Port/Starboard)
//   Helm                   → Radar
//   Comms station          → Comms
//   Sensors station        → SensorsRadar
//   Navigation station     → NavigationChart
//   Unauthorized attempt   → view_mode unchanged

import { test, expect, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';
import type { BrowserContext } from '@playwright/test';

async function waitForStation(
  client: { page: import('@playwright/test').Page; token: string },
  timeout = 5_000,
) {
  await client.page.waitForFunction(
    (t) =>
      (window as any).__messages?.some(
        (m: any) => m.type === 'StationAssigned' && m.data.token === t,
      ),
    client.token,
    { timeout },
  );
}

async function startGame(context: BrowserContext) {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  const hostId = await readHostPeerId(serverPage);

  // 9P fixed-roster layout; create enough players for each test:
  const captainPlayer = await createTestClient(context, hostId, { name: 'Captain' });
  const helmPlayer    = await createTestClient(context, hostId, { name: 'Helm' });
  const commsPlayer   = await createTestClient(context, hostId, { name: 'Comms' });
  const sensorsPlayer = await createTestClient(context, hostId, { name: 'Sensors' });
  const navPlayer     = await createTestClient(context, hostId, { name: 'Nav' });

  await captainPlayer.send('SelectStation', { station: 'Captain' });
  await waitForStation(captainPlayer);
  await helmPlayer.send('SelectStation', { station: 'Helm' });
  await waitForStation(helmPlayer);
  await commsPlayer.send('SelectStation', { station: 'Comms' });
  await waitForStation(commsPlayer);
  await sensorsPlayer.send('SelectStation', { station: 'Sensors' });
  await waitForStation(sensorsPlayer);
  await navPlayer.send('SelectStation', { station: 'Navigation' });
  await waitForStation(navPlayer);

  await captainPlayer.send('SetReady', { ready: true });
  await helmPlayer.send('SetReady', { ready: true });
  await commsPlayer.send('SetReady', { ready: true });
  await sensorsPlayer.send('SetReady', { ready: true });
  await navPlayer.send('SetReady', { ready: true });
  await captainPlayer.waitForMessage('GameStarted', 15_000);
  await helmPlayer.waitForMessage('GameStarted', 15_000);
  await commsPlayer.waitForMessage('GameStarted', 15_000);
  await sensorsPlayer.waitForMessage('GameStarted', 15_000);
  await navPlayer.waitForMessage('GameStarted', 15_000);

  return { captainPlayer, helmPlayer, commsPlayer, sensorsPlayer, navPlayer };
}

// Helper: wait for a SimState whose view_mode matches the expected value.
async function waitForViewMode(
  client: { page: import('@playwright/test').Page },
  expected: unknown,
  timeout = 5_000,
) {
  await client.page.waitForFunction(
    (exp) => {
      const msgs: any[] = (window as any).__messages ?? [];
      return msgs.some(
        (m) =>
          m.type === 'SimState' &&
          JSON.stringify(m.data.snapshot.view_mode) === JSON.stringify(exp),
      );
    },
    expected,
    { timeout },
  );
}

test('captain can set view to Camera(Fore)', async ({ context }) => {
  const { captainPlayer, helmPlayer, commsPlayer, sensorsPlayer, navPlayer } = await startGame(context);

  // Ensure we are starting from a known non-Fore view by first switching to Aft,
  // then back to Fore via the captain.
  await captainPlayer.send('SetView', { mode: { kind: 'Camera', data: 'Aft' } });
  await waitForViewMode(captainPlayer, { kind: 'Camera', data: 'Aft' });

  await captainPlayer.send('SetView', { mode: { kind: 'Camera', data: 'Fore' } });
  await waitForViewMode(captainPlayer, { kind: 'Camera', data: 'Fore' });

  await captainPlayer.close();
  await helmPlayer.close();
  await commsPlayer.close();
  await sensorsPlayer.close();
  await navPlayer.close();
});

test('captain can set view to Camera(Aft)', async ({ context }) => {
  const { captainPlayer, helmPlayer, commsPlayer, sensorsPlayer, navPlayer } = await startGame(context);

  await captainPlayer.send('SetView', { mode: { kind: 'Camera', data: 'Aft' } });
  await waitForViewMode(captainPlayer, { kind: 'Camera', data: 'Aft' });

  await captainPlayer.close();
  await helmPlayer.close();
  await commsPlayer.close();
  await sensorsPlayer.close();
  await navPlayer.close();
});

test('captain can set view to Camera(Port)', async ({ context }) => {
  const { captainPlayer, helmPlayer, commsPlayer, sensorsPlayer, navPlayer } = await startGame(context);

  await captainPlayer.send('SetView', { mode: { kind: 'Camera', data: 'Port' } });
  await waitForViewMode(captainPlayer, { kind: 'Camera', data: 'Port' });

  await captainPlayer.close();
  await helmPlayer.close();
  await commsPlayer.close();
  await sensorsPlayer.close();
  await navPlayer.close();
});

test('captain can set view to Camera(Starboard)', async ({ context }) => {
  const { captainPlayer, helmPlayer, commsPlayer, sensorsPlayer, navPlayer } = await startGame(context);

  await captainPlayer.send('SetView', { mode: { kind: 'Camera', data: 'Starboard' } });
  await waitForViewMode(captainPlayer, { kind: 'Camera', data: 'Starboard' });

  await captainPlayer.close();
  await helmPlayer.close();
  await commsPlayer.close();
  await sensorsPlayer.close();
  await navPlayer.close();
});

test('helm can set view to Radar', async ({ context }) => {
  const { captainPlayer, helmPlayer, commsPlayer, sensorsPlayer, navPlayer } = await startGame(context);

  await helmPlayer.send('SetView', { mode: { kind: 'Radar' } });
  await waitForViewMode(helmPlayer, { kind: 'Radar' });

  await captainPlayer.close();
  await helmPlayer.close();
  await commsPlayer.close();
  await sensorsPlayer.close();
  await navPlayer.close();
});

test('comms can set view to Comms', async ({ context }) => {
  const { captainPlayer, helmPlayer, commsPlayer, sensorsPlayer, navPlayer } = await startGame(context);

  await commsPlayer.send('SetView', { mode: { kind: 'Comms' } });
  await waitForViewMode(commsPlayer, { kind: 'Comms' });

  await captainPlayer.close();
  await helmPlayer.close();
  await commsPlayer.close();
  await sensorsPlayer.close();
  await navPlayer.close();
});

test('sensors can set view to SensorsRadar', async ({ context }) => {
  const { captainPlayer, helmPlayer, commsPlayer, sensorsPlayer, navPlayer } = await startGame(context);

  await sensorsPlayer.send('SetView', { mode: { kind: 'SensorsRadar' } });
  await waitForViewMode(sensorsPlayer, { kind: 'SensorsRadar' });

  await captainPlayer.close();
  await helmPlayer.close();
  await commsPlayer.close();
  await sensorsPlayer.close();
  await navPlayer.close();
});

test('navigation can set view to NavigationChart', async ({ context }) => {
  const { captainPlayer, helmPlayer, commsPlayer, sensorsPlayer, navPlayer } = await startGame(context);

  await navPlayer.send('SetView', { mode: { kind: 'NavigationChart' } });
  await waitForViewMode(navPlayer, { kind: 'NavigationChart' });

  await captainPlayer.close();
  await helmPlayer.close();
  await commsPlayer.close();
  await sensorsPlayer.close();
  await navPlayer.close();
});

test('helm cannot set view to SensorsRadar — unauthorised request is ignored', async ({
  context,
}) => {
  const { captainPlayer, helmPlayer, commsPlayer, sensorsPlayer, navPlayer } = await startGame(context);

  // Wait for initial SimState to confirm view_mode is default Camera(Fore)
  await helmPlayer.waitForMessage('SimState', 2_000);

  // Helm player attempts SensorsRadar — not authorised (Helm station only has Helm console)
  await helmPlayer.send('SetView', { mode: { kind: 'SensorsRadar' } });

  // Give the server a generous window to respond
  await helmPlayer.page.waitForTimeout(1_000);

  // No SimState with SensorsRadar should have arrived for the helm player
  const rejected = await helmPlayer.page.evaluate(() => {
    const msgs: any[] = (window as any).__messages ?? [];
    return msgs.some(
      (m) =>
        m.type === 'SimState' &&
        JSON.stringify(m.data.snapshot.view_mode) === JSON.stringify({ kind: 'SensorsRadar' }),
    );
  });
  expect(rejected).toBe(false);

  await captainPlayer.close();
  await helmPlayer.close();
  await commsPlayer.close();
  await sensorsPlayer.close();
  await navPlayer.close();
});

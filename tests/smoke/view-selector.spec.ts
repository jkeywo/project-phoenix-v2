// Issue #306 — Smoke test: SetView produces correct SimSnapshot view_mode.
//
// 2P station layout:
//   Helm station     → CaptainChair, Helm, Shields, Comms
//   Tactical station → Tactical, Repair, Power, Sensors, Navigation
//
// Authorisation rules under test:
//   Captain (CaptainChair) → Camera(Fore/Aft/Port/Starboard)
//   Helm                   → Radar
//   Comms (Helm station)   → Comms
//   Sensors (Tactical)     → SensorsRadar
//   Navigation (Tactical)  → NavigationChart
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

  // helmPlayer holds: CaptainChair, Helm, Shields, Comms
  const helmPlayer = await createTestClient(context, hostId, { name: 'Helm' });
  // tacPlayer holds: Tactical, Repair, Power, Sensors, Navigation
  const tacPlayer = await createTestClient(context, hostId, { name: 'Tac' });

  await helmPlayer.send('SelectStation', { station: 'Helm' });
  await waitForStation(helmPlayer);

  await tacPlayer.send('SelectStation', { station: 'Tactical' });
  await waitForStation(tacPlayer);

  await helmPlayer.send('StartGame');
  await helmPlayer.waitForMessage('GameStarted', 15_000);
  await tacPlayer.waitForMessage('GameStarted', 15_000);

  return { helmPlayer, tacPlayer };
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
  const { helmPlayer, tacPlayer } = await startGame(context);

  // Ensure we are starting from a known non-Fore view by first switching to Aft,
  // then back to Fore via the captain.
  await helmPlayer.send('SetView', { mode: { kind: 'Camera', data: 'Aft' } });
  await waitForViewMode(helmPlayer, { kind: 'Camera', data: 'Aft' });

  await helmPlayer.send('SetView', { mode: { kind: 'Camera', data: 'Fore' } });
  await waitForViewMode(helmPlayer, { kind: 'Camera', data: 'Fore' });

  await helmPlayer.close();
  await tacPlayer.close();
});

test('captain can set view to Camera(Aft)', async ({ context }) => {
  const { helmPlayer, tacPlayer } = await startGame(context);

  await helmPlayer.send('SetView', { mode: { kind: 'Camera', data: 'Aft' } });
  await waitForViewMode(helmPlayer, { kind: 'Camera', data: 'Aft' });

  await helmPlayer.close();
  await tacPlayer.close();
});

test('captain can set view to Camera(Port)', async ({ context }) => {
  const { helmPlayer, tacPlayer } = await startGame(context);

  await helmPlayer.send('SetView', { mode: { kind: 'Camera', data: 'Port' } });
  await waitForViewMode(helmPlayer, { kind: 'Camera', data: 'Port' });

  await helmPlayer.close();
  await tacPlayer.close();
});

test('captain can set view to Camera(Starboard)', async ({ context }) => {
  const { helmPlayer, tacPlayer } = await startGame(context);

  await helmPlayer.send('SetView', { mode: { kind: 'Camera', data: 'Starboard' } });
  await waitForViewMode(helmPlayer, { kind: 'Camera', data: 'Starboard' });

  await helmPlayer.close();
  await tacPlayer.close();
});

test('helm can set view to Radar', async ({ context }) => {
  const { helmPlayer, tacPlayer } = await startGame(context);

  await helmPlayer.send('SetView', { mode: { kind: 'Radar' } });
  await waitForViewMode(helmPlayer, { kind: 'Radar' });

  await helmPlayer.close();
  await tacPlayer.close();
});

test('comms can set view to Comms', async ({ context }) => {
  const { helmPlayer, tacPlayer } = await startGame(context);

  await helmPlayer.send('SetView', { mode: { kind: 'Comms' } });
  await waitForViewMode(helmPlayer, { kind: 'Comms' });

  await helmPlayer.close();
  await tacPlayer.close();
});

test('sensors (tactical station) can set view to SensorsRadar', async ({ context }) => {
  const { helmPlayer, tacPlayer } = await startGame(context);

  await tacPlayer.send('SetView', { mode: { kind: 'SensorsRadar' } });
  await waitForViewMode(tacPlayer, { kind: 'SensorsRadar' });

  await helmPlayer.close();
  await tacPlayer.close();
});

test('navigation (tactical station) can set view to NavigationChart', async ({ context }) => {
  const { helmPlayer, tacPlayer } = await startGame(context);

  await tacPlayer.send('SetView', { mode: { kind: 'NavigationChart' } });
  await waitForViewMode(tacPlayer, { kind: 'NavigationChart' });

  await helmPlayer.close();
  await tacPlayer.close();
});

test('helm cannot set view to SensorsRadar — unauthorised request is ignored', async ({
  context,
}) => {
  const { helmPlayer, tacPlayer } = await startGame(context);

  // Wait for initial SimState to confirm view_mode is default Camera(Fore)
  await helmPlayer.waitForMessage('SimState', 2_000);

  // Helm player attempts SensorsRadar — not authorised
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

  await helmPlayer.close();
  await tacPlayer.close();
});

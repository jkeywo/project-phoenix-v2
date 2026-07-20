// Issue #306 — Smoke test: SetView produces correct SimSnapshot view_mode.
//
// 6P fixed-roster layout (alliance_cruiser.toml):
//   Captain station  → CaptainChair
//   Helm station     → Helm
//   Comms station    → Comms + NavigationChart
//   Science station  → SensorsRadar
//
// Authorisation rules under test:
//   Captain (CaptainChair) → Camera(Fore/Aft/Port/Starboard)
//   Helm                   → Radar
//   Comms station          → Comms, NavigationChart
//   Science station        → SensorsRadar
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

  // 6P fixed-roster layout; create enough players for each test:
  const captainPlayer = await createTestClient(context, hostId, { name: 'Captain' });
  const helmPlayer    = await createTestClient(context, hostId, { name: 'Helm' });
  const sciencePlayer = await createTestClient(context, hostId, { name: 'Science' });
  const commsPlayer   = await createTestClient(context, hostId, { name: 'Comms' });

  await captainPlayer.send('SelectStation', { station: 'Captain' });
  await waitForStation(captainPlayer);
  await helmPlayer.send('SelectStation', { station: 'Helm' });
  await waitForStation(helmPlayer);
  await sciencePlayer.send('SelectStation', { station: 'Science' });
  await waitForStation(sciencePlayer);
  await commsPlayer.send('SelectStation', { station: 'Comms' });
  await waitForStation(commsPlayer);

  await captainPlayer.send('SetReady', { ready: true });
  await helmPlayer.send('SetReady', { ready: true });
  await sciencePlayer.send('SetReady', { ready: true });
  await commsPlayer.send('SetReady', { ready: true });
  await captainPlayer.waitForMessage('GameStarted', 15_000);
  await helmPlayer.waitForMessage('GameStarted', 15_000);
  await sciencePlayer.waitForMessage('GameStarted', 15_000);
  await commsPlayer.waitForMessage('GameStarted', 15_000);

  return { captainPlayer, helmPlayer, sciencePlayer, commsPlayer };
}

// Helper: wait for a BlackboardUpdate whose captain view_mode matches the
// expected value (issue #570: view_mode moved from SimSnapshot to CaptainBlackboard).
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
          m.type === 'BlackboardUpdate' &&
          m.data.updates?.some(
            ([id, bb]: [string, any]) =>
              id === 'captain' &&
              bb?.data?.view_mode != null &&
              JSON.stringify(bb.data.view_mode) === JSON.stringify(exp),
          ),
      );
    },
    expected,
    { timeout },
  );
}

test('captain can set view to Camera(Fore)', async ({ context }) => {
  const { captainPlayer, helmPlayer, sciencePlayer, commsPlayer } = await startGame(context);

  // Ensure we are starting from a known non-Fore view by first switching to Aft,
  // then back to Fore via the captain.
  await captainPlayer.send('ControlSystem', { target: 'viewscreen', payload: { type: 'SetView', data: { mode: { kind: 'Camera', data: 'Aft' } } } });
  await waitForViewMode(captainPlayer, { kind: 'Camera', data: 'Aft' });

  await captainPlayer.send('ControlSystem', { target: 'viewscreen', payload: { type: 'SetView', data: { mode: { kind: 'Camera', data: 'Fore' } } } });
  await waitForViewMode(captainPlayer, { kind: 'Camera', data: 'Fore' });

  await captainPlayer.close();
  await helmPlayer.close();
  await commsPlayer.close();
  await sciencePlayer.close();
});

test('captain can set view to Camera(Aft)', async ({ context }) => {
  const { captainPlayer, helmPlayer, sciencePlayer, commsPlayer } = await startGame(context);

  await captainPlayer.send('ControlSystem', { target: 'viewscreen', payload: { type: 'SetView', data: { mode: { kind: 'Camera', data: 'Aft' } } } });
  await waitForViewMode(captainPlayer, { kind: 'Camera', data: 'Aft' });

  await captainPlayer.close();
  await helmPlayer.close();
  await commsPlayer.close();
  await sciencePlayer.close();
});

test('captain can set view to Camera(Port)', async ({ context }) => {
  const { captainPlayer, helmPlayer, sciencePlayer, commsPlayer } = await startGame(context);

  await captainPlayer.send('ControlSystem', { target: 'viewscreen', payload: { type: 'SetView', data: { mode: { kind: 'Camera', data: 'Port' } } } });
  await waitForViewMode(captainPlayer, { kind: 'Camera', data: 'Port' });

  await captainPlayer.close();
  await helmPlayer.close();
  await commsPlayer.close();
  await sciencePlayer.close();
});

test('captain can set view to Camera(Starboard)', async ({ context }) => {
  const { captainPlayer, helmPlayer, sciencePlayer, commsPlayer } = await startGame(context);

  await captainPlayer.send('ControlSystem', { target: 'viewscreen', payload: { type: 'SetView', data: { mode: { kind: 'Camera', data: 'Starboard' } } } });
  await waitForViewMode(captainPlayer, { kind: 'Camera', data: 'Starboard' });

  await captainPlayer.close();
  await helmPlayer.close();
  await commsPlayer.close();
  await sciencePlayer.close();
});

test('helm can set view to Radar', async ({ context }) => {
  const { captainPlayer, helmPlayer, sciencePlayer, commsPlayer } = await startGame(context);

  await helmPlayer.send('ControlSystem', { target: 'viewscreen', payload: { type: 'SetView', data: { mode: { kind: 'Radar' } } } });
  await waitForViewMode(helmPlayer, { kind: 'Radar' });

  await captainPlayer.close();
  await helmPlayer.close();
  await commsPlayer.close();
  await sciencePlayer.close();
});

test('comms can set view to Comms', async ({ context }) => {
  const { captainPlayer, helmPlayer, sciencePlayer, commsPlayer } = await startGame(context);

  await commsPlayer.send('ControlSystem', { target: 'viewscreen', payload: { type: 'SetView', data: { mode: { kind: 'Comms' } } } });
  await waitForViewMode(commsPlayer, { kind: 'Comms' });

  await captainPlayer.close();
  await helmPlayer.close();
  await commsPlayer.close();
  await sciencePlayer.close();
});

test('science can set view to SensorsRadar', async ({ context }) => {
  const { captainPlayer, helmPlayer, sciencePlayer, commsPlayer } = await startGame(context);

  await sciencePlayer.send('ControlSystem', { target: 'viewscreen', payload: { type: 'SetView', data: { mode: { kind: 'SensorsRadar' } } } });
  await waitForViewMode(sciencePlayer, { kind: 'SensorsRadar' });

  await captainPlayer.close();
  await helmPlayer.close();
  await commsPlayer.close();
  await sciencePlayer.close();
});

test('comms can set view to NavigationChart', async ({ context }) => {
  const { captainPlayer, helmPlayer, sciencePlayer, commsPlayer } = await startGame(context);

  await commsPlayer.send('ControlSystem', { target: 'viewscreen', payload: { type: 'SetView', data: { mode: { kind: 'NavigationChart' } } } });
  await waitForViewMode(commsPlayer, { kind: 'NavigationChart' });

  await captainPlayer.close();
  await helmPlayer.close();
  await commsPlayer.close();
  await sciencePlayer.close();
});

test('helm cannot set view to SensorsRadar — unauthorised request is ignored', async ({
  context,
}) => {
  const { captainPlayer, helmPlayer, sciencePlayer, commsPlayer } = await startGame(context);

  // Wait for a BlackboardUpdate with captain blackboard to confirm the
  // default view_mode is Camera(Fore).
  await helmPlayer.page.waitForFunction(
    () => (window as any).__messages?.some(
      (m) => m.type === 'BlackboardUpdate' && m.data.updates?.some(
        ([id]: [string, any]) => id === 'captain',
      ),
    ),
    undefined,
    { timeout: 5_000 },
  );

  // Helm player attempts SensorsRadar — not authorised (Helm station only has Helm console)
  await helmPlayer.send('ControlSystem', { target: 'viewscreen', payload: { type: 'SetView', data: { mode: { kind: 'SensorsRadar' } } } });

  // Give the server a generous window to respond
  await helmPlayer.page.waitForTimeout(1_000);

  // No BlackboardUpdate with SensorsRadar view_mode should have arrived
  const rejected = await helmPlayer.page.evaluate(() => {
    const msgs: any[] = (window as any).__messages ?? [];
    return msgs.some(
      (m) =>
        m.type === 'BlackboardUpdate' &&
        m.data.updates?.some(
          ([id, bb]: [string, any]) =>
            id === 'captain' &&
            bb?.data?.view_mode != null &&
            JSON.stringify(bb.data.view_mode) === JSON.stringify({ kind: 'SensorsRadar' }),
        ),
    );
  });
  expect(rejected).toBe(false);

  await captainPlayer.close();
  await helmPlayer.close();
  await commsPlayer.close();
  await sciencePlayer.close();
});

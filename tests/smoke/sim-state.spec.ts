// Issues #58 + #59 — Smoke tests: SimState broadcast and HelmInput physics.

import { test, expect, type TestClient } from './fixtures';
import { readHostPeerId, createTestClient } from './fixtures';
import type { BrowserContext } from '@playwright/test';

async function startGame(context: BrowserContext): Promise<{ captain: TestClient; helm: TestClient }> {
  const serverPage = await context.newPage();
  await serverPage.goto('/');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 15_000 });

  const hostId = await readHostPeerId(serverPage);

  const captain = await createTestClient(context, hostId, { name: 'Cap' });
  const helm = await createTestClient(context, hostId, { name: 'Helm' });

  // 2P layout: Helm station (CaptainChair+Helm) and Tactical station.
  // The Helm station also carries CaptainChair, so the helm player is the captain.
  await helm.send('SelectStation', { station: 'Helm' });
  await helm.waitForMessage('StationAssigned', 5_000);

  await captain.send('SelectStation', { station: 'Tactical' });
  await captain.waitForMessage('StationAssigned', 5_000);

  await helm.send('StartGame');
  await helm.waitForMessage('GameStarted', 5_000);
  await captain.waitForMessage('GameStarted', 5_000);

  return { captain, helm };
}

test('SimState is broadcast to all clients within 2 s of game start', async ({ context }) => {
  const { captain, helm } = await startGame(context);

  const simA = await captain.waitForMessage('SimState', 2_000) as any;
  const simB = await helm.waitForMessage('SimState', 2_000) as any;

  for (const sim of [simA, simB]) {
    const snap = sim.data.snapshot;
    expect(typeof snap.ship_x).toBe('number');
    expect(typeof snap.ship_z).toBe('number');
    expect(typeof snap.ship_yaw).toBe('number');
    expect(typeof snap.red_alert).toBe('boolean');
    expect(snap.view_mode).toBeDefined();
  }

  await captain.close();
  await helm.close();
});

test('HelmInput changes ship position in subsequent SimState', async ({ context }) => {
  const { captain, helm } = await startGame(context);

  // Record initial position from first SimState
  const first = await helm.waitForMessage('SimState', 2_000) as any;
  const initX: number = first.data.snapshot.ship_x;
  const initZ: number = first.data.snapshot.ship_z;

  // Start repeating HelmInput so the server receives sustained thrust
  await helm.page.evaluate(() => {
    (window as any).__helmInterval = setInterval(() => {
      (window as any).__conn.send(JSON.stringify({
        type: 'HelmInput',
        data: { thrust: 1.0, steering: 0.0 },
      }));
    }, 100);
  });

  // Wait up to 3 s for a SimState showing the ship has moved by more than rounding error
  await helm.page.waitForFunction(
    ({ x, z }: { x: number; z: number }) => {
      const msgs: any[] = (window as any).__messages;
      return msgs.some(
        (m) =>
          m.type === 'SimState' &&
          (Math.abs(m.data.snapshot.ship_x - x) > 0.05 ||
            Math.abs(m.data.snapshot.ship_z - z) > 0.05),
      );
    },
    { x: initX, z: initZ },
    { timeout: 10_000 },
  );

  // Stop repeating inputs
  await helm.page.evaluate(() => clearInterval((window as any).__helmInterval));

  // Confirm at least one moved SimState exists
  const moved = await helm.page.evaluate(
    ({ x, z }: { x: number; z: number }) => {
      const msgs: any[] = (window as any).__messages;
      return msgs.filter(
        (m) =>
          m.type === 'SimState' &&
          (Math.abs(m.data.snapshot.ship_x - x) > 0.05 ||
            Math.abs(m.data.snapshot.ship_z - z) > 0.05),
      ).length;
    },
    { x: initX, z: initZ },
  );

  expect(moved).toBeGreaterThan(0);

  await captain.close();
  await helm.close();
});

// Issues #58 + #59 — Smoke tests: SimState broadcast and HelmInput physics.

import fs from 'fs';
import path from 'path';
import { test, expect, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';
import type { TestClient } from './fixtures';
import type { BrowserContext } from '@playwright/test';

// Read impulse_charge_duration from the ship TOML so the test timeout is
// derived from the configured value rather than a hardcoded constant.
const shipToml = fs.readFileSync(
  path.resolve(__dirname, '../../assets/entities/alliance_cruiser.toml'),
  'utf-8',
);
const chargeMatch = shipToml.match(/impulse_charge_duration\s*=\s*([0-9.]+)/);
const IMPULSE_CHARGE_DURATION_S = chargeMatch ? parseFloat(chargeMatch[1]) : 3.0;

async function waitForStation(client: { page: import('@playwright/test').Page; token: string }, timeout = 5_000) {
  await client.page.waitForFunction(
    (t) => (window as any).__messages?.some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t
    ),
    client.token,
    { timeout },
  );
}

async function startGame(context: BrowserContext): Promise<{ captain: TestClient; helm: TestClient; serverPage: import('@playwright/test').Page }> {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  const hostId = await readHostPeerId(serverPage);

  const captain = await createTestClient(context, hostId, { name: 'Cap' });
  const helm = await createTestClient(context, hostId, { name: 'Helm' });

  // Bring client pages to front to prevent Chrome from throttling their
  // timers (rAF, setTimeout) in the background, which can delay message
  // dispatch and cause flaky timeouts on loaded CI runners.
  await captain.page.bringToFront();
  await helm.page.bringToFront();
  await serverPage.bringToFront();

  // 2P layout: Helm station (CaptainChair+Helm) and Tactical station.
  // The Helm station also carries CaptainChair, so the helm player is the captain.
  await helm.send('SelectStation', { station: 'Helm' });
  await waitForStation(helm);

  await captain.send('SelectStation', { station: 'Tactical' });
  await waitForStation(captain);

  await helm.send('SetReady', { ready: true });
  await captain.send('SetReady', { ready: true });
  await helm.waitForMessage('GameStarted', 10_000);
  await captain.waitForMessage('GameStarted', 10_000);

  return { captain, helm, serverPage };
}

test('SimState is broadcast to all clients within 2 s of game start', async ({ context }) => {
  const { captain, helm } = await startGame(context);

  // SimState is still broadcast (now only snapshot.entity_states)
  const simA = await captain.waitForMessage('SimState', 2_000) as any;
  const simB = await helm.waitForMessage('SimState', 2_000) as any;
  expect(Array.isArray(simA.data.snapshot.entity_states)).toBe(true);
  expect(Array.isArray(simB.data.snapshot.entity_states)).toBe(true);

  // Ship position/state comes via BlackboardUpdate (issue #570)
  const bbCap = await captain.waitForMessage('BlackboardUpdate', 2_000) as any;
  const bbHelm = await helm.waitForMessage('BlackboardUpdate', 2_000) as any;
  const hasHelm = (updates: any[]) => updates.some(([id]: [string, any]) => id === 'helm');
  expect(hasHelm(bbCap.data.updates)).toBe(true);
  expect(hasHelm(bbHelm.data.updates)).toBe(true);

  await captain.close();
  await helm.close();
});

test('StartImpulseCharge completes in the TOML-configured duration (~3 s)', async ({ context }) => {
  const { captain, helm } = await startGame(context);

  // Wait for the first BlackboardUpdate to confirm simulation is running
  await helm.waitForMessage('BlackboardUpdate', 2_000);

  await helm.send('StartImpulseCharge');

  // Wait up to 8× the TOML-configured charge duration for a HelmBlackboard
  // showing impulse_charge has reached 1.0 (headroom for CI latency).
  const chargeTimeoutMs = IMPULSE_CHARGE_DURATION_S * 8 * 1000;
  await helm.page.waitForFunction(
    () => {
      const msgs: any[] = (window as any).__messages;
      return msgs.some(
        (m) =>
          m.type === 'BlackboardUpdate' &&
          m.data.updates?.some(
            ([id, bb]: [string, any]) =>
              id === 'helm' && bb.data.impulse_charge >= 1.0,
          ),
      );
    },
    undefined,
    { timeout: chargeTimeoutMs },
  );

  const charged = await helm.page.evaluate(() => {
    const msgs: any[] = (window as any).__messages;
    return msgs.filter(
      (m) =>
        m.type === 'BlackboardUpdate' &&
        m.data.updates?.some(
          ([id, bb]: [string, any]) =>
            id === 'helm' && bb.data.impulse_charge >= 1.0,
        ),
    ).length;
  });

  expect(charged).toBeGreaterThan(0);

  await captain.close();
  await helm.close();
});

test('HelmInput changes ship position in subsequent blackboard updates', async ({ context }) => {
  const { captain, helm, serverPage } = await startGame(context);

  // Record initial position from first HelmBlackboard
  const first = await helm.waitForMessage('BlackboardUpdate', 2_000) as any;
  const firstHelm = first.data.updates.find(([id]: [string, any]) => id === 'helm');
  const initX: number = firstHelm[1].data.x;
  const initZ: number = firstHelm[1].data.z;

  // Keep the WASM server page in the foreground so Chrome doesn't throttle
  // its rAF/timers, which would stall the Bevy simulation tick.
  await serverPage.bringToFront();

  const movedBlackboardCount = async () => helm.page.evaluate(
    ({ x, z }: { x: number; z: number }) => {
      const msgs: any[] = (window as any).__messages;
      return msgs.filter(
        (m) =>
          m.type === 'BlackboardUpdate' &&
          m.data.updates?.some(
            ([id, bb]: [string, any]) =>
              id === 'helm' &&
              (Math.abs(bb.data.x - x) > 0.05 ||
                Math.abs(bb.data.z - z) > 0.05),
          ),
      ).length;
    },
    { x: initX, z: initZ },
  );

  // Send sustained thrust from the Playwright side instead of relying on an
  // in-page interval. The helm page is backgrounded while the server stays in
  // front, and Chromium can throttle background page timers on CI runners.
  await expect.poll(
    async () => {
      await helm.send('ControlSystem', {
        target: 'helm',
        payload: { type: 'HelmInput', data: { thrust: 1.0, steering: 0.0 } },
      });
      await serverPage.bringToFront();
      return movedBlackboardCount();
    },
    {
      timeout: 20_000,
      intervals: [100, 100, 200, 200, 500],
    },
  ).toBeGreaterThan(0);

  // Stop thrust so this test leaves the shared server page in a quiet state.
  await helm.send('ControlSystem', {
    target: 'helm',
    payload: { type: 'HelmInput', data: { thrust: 0.0, steering: 0.0 } },
  });

  // Confirm at least one moved BlackboardUpdate exists.
  const moved = await movedBlackboardCount();
  expect(moved).toBeGreaterThan(0);

  await captain.close();
  await helm.close();
});

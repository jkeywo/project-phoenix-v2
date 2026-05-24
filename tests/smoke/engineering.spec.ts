// Issue #70 — Smoke test: Per-console hull integrity in SimState.
//
// SimSnapshot no longer carries a flat hull_integrity; it carries
// console_hull: Vec<ConsoleHullStatus { console, current, max_hp }>.

import { test, expect, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';
import type { BrowserContext } from '@playwright/test';

async function waitForStation(client: { page: import('@playwright/test').Page; token: string }, timeout = 5_000) {
  await client.page.waitForFunction(
    (t) => (window as any).__messages?.some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t
    ),
    client.token,
    { timeout },
  );
}

async function startGameWithEngineering(context: BrowserContext) {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  const hostId = await readHostPeerId(serverPage);

  const captain = await createTestClient(context, hostId, { name: 'Cap' });
  const engineer = await createTestClient(context, hostId, { name: 'Eng' });

  // 2P layout: Helm (CaptainChair+Helm) + Tactical (Tactical+Repair)
  await captain.send('SelectStation', { station: 'Helm' });
  await waitForStation(captain);

  await engineer.send('SelectStation', { station: 'Tactical' });
  await waitForStation(engineer);

  await captain.send('StartGame');
  await captain.waitForMessage('GameStarted', 5_000);
  await engineer.waitForMessage('GameStarted', 5_000);

  return { captain, engineer };
}

test('Engineering player receives console_hull in SimState after game start', async ({ context }) => {
  const { captain, engineer } = await startGameWithEngineering(context);

  const simState = await engineer.waitForMessage('SimState', 2_000) as any;
  const hull = simState.data.snapshot.console_hull;

  expect(Array.isArray(hull)).toBe(true);
  expect(hull.length).toBeGreaterThanOrEqual(1);
  for (const entry of hull) {
    expect(typeof entry.console).toBe('string');
    expect(typeof entry.current).toBe('number');
    expect(typeof entry.max_hp).toBe('number');
    expect(entry.current).toBeGreaterThanOrEqual(0);
    expect(entry.current).toBeLessThanOrEqual(entry.max_hp);
  }

  await captain.close();
  await engineer.close();
});

test('total hull starts at 100 in first SimState', async ({ context }) => {
  const { captain, engineer } = await startGameWithEngineering(context);

  const simState = await engineer.waitForMessage('SimState', 2_000) as any;
  const hull = simState.data.snapshot.console_hull;
  const total = hull.reduce((sum: number, e: any) => sum + e.current, 0);

  expect(total).toBe(100);

  await captain.close();
  await engineer.close();
});
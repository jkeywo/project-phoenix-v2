// Issue #70 — Smoke test: Per-system hull integrity in SimState.
//
// SimSnapshot no longer carries a flat hull_integrity; it carries
// system_hull: Vec<SystemHullStatus { system_id, display_name, current, max_hp, ... }>
// broadcast via `SystemHullUpdate` (issue #618, renamed from
// `ConsoleHullUpdate` when the publisher stopped emitting legacy
// Console-keyed wire fields).

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

  await captain.send('SetReady', { ready: true });
  await engineer.send('SetReady', { ready: true });
  await captain.waitForMessage('GameStarted', 10_000);
  await engineer.waitForMessage('GameStarted', 10_000);

  return { captain, engineer };
}

test('Engineering player receives SystemHullUpdate after game start', async ({ context }) => {
  const { captain, engineer } = await startGameWithEngineering(context);

  const msg = await engineer.waitForMessage('SystemHullUpdate', 2_000) as any;
  const hull = msg.data.entries;

  expect(Array.isArray(hull)).toBe(true);
  expect(hull.length).toBeGreaterThanOrEqual(1);
  for (const entry of hull) {
    expect(typeof entry.system_id).toBe('string');
    expect(typeof entry.display_name).toBe('string');
    expect(typeof entry.current).toBe('number');
    expect(typeof entry.max_hp).toBe('number');
    expect(entry.current).toBeGreaterThanOrEqual(0);
    expect(entry.current).toBeLessThanOrEqual(entry.max_hp);
  }

  await captain.close();
  await engineer.close();
});

test('total hull starts at 211 in first SystemHullUpdate', async ({ context }) => {
  const { captain, engineer } = await startGameWithEngineering(context);

  const msg = await engineer.waitForMessage('SystemHullUpdate', 2_000) as any;
  const hull = msg.data.entries;
  const total = hull.reduce((sum: number, e: any) => sum + e.current, 0);

  // 150 (post-#511) + 86 (fine Tactical banks/tubes/magazine added in #512,
  // alongside the retained coarse "Tactical" entry) - 25 (Shields hull moved
  // out of console_hull into per-arc ShipArcHull in #514)
  // + 20 (alliance_battleship/destroyer hull added in #639/#640/#641) = 231.
  expect(total).toBe(231);

  await captain.close();
  await engineer.close();
});
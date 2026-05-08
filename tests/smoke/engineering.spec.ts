// Issue #70 — Smoke test: Engineering console receives hull_integrity in SimState.

import { test, expect } from './fixtures';
import { readHostPeerId, createTestClient } from './fixtures';
import type { BrowserContext } from '@playwright/test';

async function startGameWithEngineering(context: BrowserContext) {
  const serverPage = await context.newPage();
  await serverPage.goto('/');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 15_000 });

  const hostId = await readHostPeerId(serverPage);

  const captain = await createTestClient(context, hostId, { name: 'Cap' });
  const engineer = await createTestClient(context, hostId, { name: 'Eng' });

  await captain.send('SelectConsole', { console: 'CaptainChair' });
  await captain.waitForMessage('ConsoleSelected', 5_000);

  await engineer.send('SelectConsole', { console: 'Engineering' });
  await engineer.waitForMessage('ConsoleSelected', 5_000);

  await captain.send('StartGame');
  await captain.waitForMessage('GameStarted', 5_000);
  await engineer.waitForMessage('GameStarted', 5_000);

  return { captain, engineer };
}

test('Engineering player receives hull_integrity in SimState after game start', async ({ context }) => {
  const { captain, engineer } = await startGameWithEngineering(context);

  const simState = await engineer.waitForMessage('SimState', 2_000) as any;
  const snap = simState.data.snapshot;

  expect(typeof snap.hull_integrity).toBe('number');
  expect(snap.hull_integrity).toBeGreaterThanOrEqual(0);
  expect(snap.hull_integrity).toBeLessThanOrEqual(100);

  await captain.close();
  await engineer.close();
});

test('hull_integrity starts at 100 in first SimState', async ({ context }) => {
  const { captain, engineer } = await startGameWithEngineering(context);

  const simState = await engineer.waitForMessage('SimState', 2_000) as any;
  expect(simState.data.snapshot.hull_integrity).toBe(100);

  await captain.close();
  await engineer.close();
});

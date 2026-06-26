// Regression: Power and Repair stations must honor actions from their
// holders. With the fixed-roster model (#495), each station holds exactly
// one console — Power manages allocation, Repair dispatches teams.
//
// Power/Repair both authorize taps against `console_holder(X)` — the same
// function that decides who receives state. So the failure mode we guard
// against is a token/holder desync: a client believes it holds a station
// (and receives its state) while the server rejects its actions.

import { test, expect } from './fixtures';
import { readHostPeerId, createServerPage, createTestClient } from './fixtures';
import type { TestClient } from './fixtures';

/** Send SelectStation and wait for a StationAssigned for *this* client's token. */
async function selectAndWait(client: TestClient, station: string, timeout = 5_000) {
  await client.send('SelectStation', { station });
  await client.page.waitForFunction(
    (t) => (window as any).__messages?.some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t,
    ),
    client.token,
    { timeout },
  );
}

/** The last StationAssigned received for a token (or null). */
async function lastAssignment(client: TestClient, token: string) {
  return client.page.evaluate(
    (t) => {
      const msgs: any[] = (window as any).__messages || [];
      const all = msgs.filter((m: any) => m.type === 'StationAssigned' && m.data.token === t);
      return all.length > 0 ? all[all.length - 1] : null;
    },
    token,
  ) as Promise<any>;
}

/** Wait until the last message of `type` satisfies `predicate` (run in-page). */
async function waitForLastMessage(
  client: TestClient,
  type: string,
  predicate: string,
  timeout = 5_000,
) {
  await client.page.waitForFunction(
    ({ type, predicate }) => {
      const msgs: any[] = (window as any).__messages || [];
      const last = msgs.filter((m: any) => m.type === type).pop();
      if (!last) return false;
      // eslint-disable-next-line no-new-func
      return new Function('data', `return (${predicate})`)(last.data);
    },
    { type, predicate },
    { timeout },
  );
}

async function setHelmPower(client: TestClient, level: number) {
  await client.send('ControlSystem', {
    target: 'power',
    payload: {
      type: 'SetPowerGroupAllocation',
      data: { group: 'helm', level },
    },
  });
}

/**
 * Build a 4-player crew at the fixed 6P layout. Returns four clients;
 * c3 is the Power station under test for power-related tests.
 */
async function buildFourPlayerCrew(context: import('@playwright/test').BrowserContext) {
  const serverPage = await createServerPage(context);
  const hostId = await readHostPeerId(serverPage);

  const c1 = await createTestClient(context, hostId, { name: 'P1' });
  await selectAndWait(c1, 'Captain');

  const c2 = await createTestClient(context, hostId, { name: 'P2' });
  await selectAndWait(c2, 'Helm');

  const c3 = await createTestClient(context, hostId, { name: 'Eng' });
  await selectAndWait(c3, 'Power');

  const c4 = await createTestClient(context, hostId, { name: 'Sci' });
  await selectAndWait(c4, 'Sensors');

  return { c1, c2, c3, c4, hostId };
}

test('Power station is assigned correctly at 6P layout', async ({ context }) => {
  const { c1, c2, c3, c4 } = await buildFourPlayerCrew(context);

  const a3 = await lastAssignment(c3, c3.token);
  expect(a3.data.station).toBe('Power');
  expect([...a3.data.consoles].sort()).toEqual(['Power']);

  await c1.close();
  await c2.close();
  await c3.close();
  await c4.close();
});

test('Power player can change helm allocation', async ({ context }) => {
  const { c1, c2, c3, c4 } = await buildFourPlayerCrew(context);

  await c1.send('SetReady', { ready: true });
  await c2.send('SetReady', { ready: true });
  await c3.send('SetReady', { ready: true });
  await c4.send('SetReady', { ready: true });
  await c3.waitForMessage('GameStarted', 5_000);

  await waitForLastMessage(c3, 'PowerState', 'data && data.helm === 2');

  await setHelmPower(c3, 3);
  await waitForLastMessage(c3, 'PowerState', 'data && data.helm === 3');

  await c1.close();
  await c2.close();
  await c3.close();
  await c4.close();
});

test('Repair player can dispatch a repair team', async ({ context }) => {
  const serverPage = await createServerPage(context);
  const hostId = await readHostPeerId(serverPage);

  const c1 = await createTestClient(context, hostId, { name: 'P1' });
  await selectAndWait(c1, 'Captain');

  const c2 = await createTestClient(context, hostId, { name: 'P2' });
  await selectAndWait(c2, 'Helm');

  const c3 = await createTestClient(context, hostId, { name: 'Eng' });
  await selectAndWait(c3, 'Repair');

  const c4 = await createTestClient(context, hostId, { name: 'Sci' });
  await selectAndWait(c4, 'Sensors');

  await c1.send('SetReady', { ready: true });
  await c2.send('SetReady', { ready: true });
  await c3.send('SetReady', { ready: true });
  await c4.send('SetReady', { ready: true });
  await c3.waitForMessage('GameStarted', 5_000);

  await waitForLastMessage(
    c3,
    'RepairState',
    'data && Array.isArray(data.teams) && data.teams[0] === "Idle"',
  );

  await c3.send('DispatchRepairTeam', { team_idx: 0, console: 'Power' });
  await waitForLastMessage(
    c3,
    'RepairState',
    'data && data.teams[0] !== "Idle"',
  );

  await c1.close();
  await c2.close();
  await c3.close();
  await c4.close();
});

test('Power acts when all four connect before selecting', async ({ context }) => {
  const serverPage = await createServerPage(context);
  const hostId = await readHostPeerId(serverPage);

  const c1 = await createTestClient(context, hostId, { name: 'P1' });
  const c2 = await createTestClient(context, hostId, { name: 'P2' });
  const c3 = await createTestClient(context, hostId, { name: 'Eng' });
  const c4 = await createTestClient(context, hostId, { name: 'Sci' });

  await selectAndWait(c1, 'Captain');
  await selectAndWait(c2, 'Helm');
  await selectAndWait(c3, 'Power');
  await selectAndWait(c4, 'Sensors');

  const a3 = await lastAssignment(c3, c3.token);
  expect([...a3.data.consoles].sort()).toEqual(['Power']);

  await c1.send('SetReady', { ready: true });
  await c2.send('SetReady', { ready: true });
  await c3.send('SetReady', { ready: true });
  await c4.send('SetReady', { ready: true });
  await c3.waitForMessage('GameStarted', 5_000);

  await waitForLastMessage(c3, 'PowerState', 'data && data.helm === 2');

  await setHelmPower(c3, 3);
  await waitForLastMessage(c3, 'PowerState', 'data && data.helm === 3');

  await c1.close();
  await c2.close();
  await c3.close();
  await c4.close();
});

test('Power can still act after a mid-game reconnect', async ({ context }) => {
  const { c1, c2, c3, c4, hostId } = await buildFourPlayerCrew(context);
  const powToken = c3.token;

  await c1.send('SetReady', { ready: true });
  await c2.send('SetReady', { ready: true });
  await c3.send('SetReady', { ready: true });
  await c4.send('SetReady', { ready: true });
  await c3.waitForMessage('GameStarted', 5_000);

  await waitForLastMessage(c3, 'PowerState', 'data && data.helm === 2');
  await setHelmPower(c3, 3);
  await waitForLastMessage(c3, 'PowerState', 'data && data.helm === 3');

  await c3.close();
  const c3b = await createTestClient(context, hostId, { token: powToken, name: 'Eng' });

  // Wait for any PowerState (AI may have reset helm during disconnect).
  await waitForLastMessage(c3b, 'PowerState', 'data && typeof data.helm === "number"', 10_000);
  // Restore helm=3 in case AI changed it, then verify.
  await setHelmPower(c3b, 3);
  await waitForLastMessage(c3b, 'PowerState', 'data && data.helm === 3', 10_000);

  await setHelmPower(c3b, 2);
  await waitForLastMessage(c3b, 'PowerState', 'data && data.helm === 2', 10_000);

  await c1.close();
  await c2.close();
  await c3b.close();
  await c4.close();
});

test('shared session-token orphans the first Power device (ghost console)', async ({ context }) => {
  const { c1, c2, c3, c4, hostId } = await buildFourPlayerCrew(context);
  const powToken = c3.token;

  await c1.send('SetReady', { ready: true });
  await c2.send('SetReady', { ready: true });
  await c3.send('SetReady', { ready: true });
  await c4.send('SetReady', { ready: true });
  await c3.waitForMessage('GameStarted', 5_000);

  await waitForLastMessage(c3, 'PowerState', 'data && data.helm === 2');

  await setHelmPower(c3, 3);
  await waitForLastMessage(c3, 'PowerState', 'data && data.helm === 3');

  // Record c3's message count before ghost connects
  const preCount = await c3.page.evaluate(() => (window as any).__messages?.length ?? 0);

  const ghostWinner = await createTestClient(context, hostId, { token: powToken, name: 'Eng-2' });
  await waitForLastMessage(ghostWinner, 'PowerState', 'data && typeof data.helm === "number"');

  // ghostWinner sends a change so we can verify c3 (ghost) does NOT receive updates
  await setHelmPower(ghostWinner, 2);
  await waitForLastMessage(ghostWinner, 'PowerState', 'data && data.helm === 2');

  // Small settling window to drain any in-flight SimState (the server tick
  // that might have fired between tokenConns overwrite and this check).
  await c3.page.waitForTimeout(500);

  const sawNewPower = await c3.page.evaluate((count) => {
    const msgs: any[] = (window as any).__messages || [];
    const newMsgs = msgs.slice(count);
    return newMsgs.some((m: any) => m.type === 'PowerState');
  }, preCount);
  expect(sawNewPower).toBe(false);

  await c1.close();
  await c2.close();
  await c3.close();
  await ghostWinner.close();
  await c4.close();
});

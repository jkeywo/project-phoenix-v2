// Regression: Engineering station must honor power and repair actions
// from its holder. With the fixed-roster model (#495), each station holds
// exactly one console — Engineering manages power allocation and repair
// teams.
//
// Both power and repair actions authorize taps against
// `console_holder(X)` — the same function that decides who receives state.
// So the failure mode we guard against is a token/holder desync: a client
// believes it holds a station (and receives its state) while the server
// rejects its actions.

import fs from 'fs';
import path from 'path';
import { test, expect, tomlNumber } from './fixtures';
import { readHostPeerId, createServerPage, createTestClient } from './fixtures';
import type { TestClient } from './fixtures';

// The hull `MINIMAL_DEFAULT_WORLD` spawns as the player ship (see fixtures.ts).
// Only its *legal power range* is read from here — the level the ship boots at
// is taken off the wire, not from this file — so a rebalance of the authored
// defaults cannot break these tests. Same derive-don't-pin rule as
// `sim-state.spec.ts`, per issue #941.
const PLAYER_HULL_TOML = fs.readFileSync(
  path.resolve(__dirname, '../../assets/entities/alliance_cruiser.toml'),
  'utf-8',
);
const HELM_POWER_MAX = tomlNumber(PLAYER_HULL_TOML, 'power_groups.helm', 'max_level');
const HELM_POWER_MIN = tomlNumber(PLAYER_HULL_TOML, 'power_groups.helm', 'min_level');

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
    target: 'power-reactor',
    payload: {
      type: 'SetPowerGroupAllocation',
      data: { group: 'helm', level },
    },
  });
}

/** The helm allocation the ship boots with, read off the first PowerState.
 *
 *  Issue #941: these tests used to wait for `data.helm === 2` — the level the
 *  player hull happened to author for the helm power group. A power rebalance
 *  moved it and broke a test about *authorisation*, which is what the taps
 *  below are actually checking. Reading the boot level and then moving off it
 *  keeps the real assertion (the Engineering holder's tap is honoured) and
 *  drops the authored constant.
 */
async function bootHelmPower(client: TestClient): Promise<number> {
  await waitForLastMessage(client, 'PowerState', 'data && typeof data.helm === "number"');
  const msg = await client.lastMessage('PowerState') as any;
  return msg.data.helm as number;
}

/** A helm level that is not `from` and is inside the hull's authored range. */
function otherLevel(from: number): number {
  const other = from < HELM_POWER_MAX ? from + 1 : from - 1;
  if (other < HELM_POWER_MIN || other > HELM_POWER_MAX || other === from) {
    throw new Error(
      `helm power group has no second level to move to (min=${HELM_POWER_MIN}, ` +
        `max=${HELM_POWER_MAX}, at=${from}) — these tests need one`,
    );
  }
  return other;
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
  await selectAndWait(c3, 'Engineering');

  const c4 = await createTestClient(context, hostId, { name: 'Sci' });
  await selectAndWait(c4, 'Science');

  return { c1, c2, c3, c4, hostId };
}

test('Engineering station is assigned correctly at 6P layout', async ({ context }) => {
  const { c1, c2, c3, c4 } = await buildFourPlayerCrew(context);

  const a3 = await lastAssignment(c3, c3.token);
  expect(a3.data.station).toBe('Engineering');
  expect(a3.data.station_id).toBe('engineering');

  await c1.close();
  await c2.close();
  await c3.close();
  await c4.close();
});

test('Engineering player can change helm allocation', async ({ context }) => {
  const { c1, c2, c3, c4 } = await buildFourPlayerCrew(context);

  await c1.send('SetReady', { ready: true });
  await c2.send('SetReady', { ready: true });
  await c3.send('SetReady', { ready: true });
  await c4.send('SetReady', { ready: true });
  await c3.waitForMessage('GameStarted', 10_000);

  const boot = await bootHelmPower(c3);
  const moved = otherLevel(boot);
  await setHelmPower(c3, moved);
  await waitForLastMessage(c3, 'PowerState', `data && data.helm === ${moved}`);

  // And back — a one-way move could be an AI nudge rather than this client's
  // tap being honoured; a round trip to a level it just left cannot be.
  await setHelmPower(c3, boot);
  await waitForLastMessage(c3, 'PowerState', `data && data.helm === ${boot}`);

  await c1.close();
  await c2.close();
  await c3.close();
  await c4.close();
});

test('Engineering player can dispatch a repair team', async ({ context }) => {
  const serverPage = await createServerPage(context);
  const hostId = await readHostPeerId(serverPage);

  const c1 = await createTestClient(context, hostId, { name: 'P1' });
  await selectAndWait(c1, 'Captain');

  const c2 = await createTestClient(context, hostId, { name: 'P2' });
  await selectAndWait(c2, 'Helm');

  const c3 = await createTestClient(context, hostId, { name: 'Eng' });
  await selectAndWait(c3, 'Engineering');

  const c4 = await createTestClient(context, hostId, { name: 'Sci' });
  await selectAndWait(c4, 'Science');

  await c1.send('SetReady', { ready: true });
  await c2.send('SetReady', { ready: true });
  await c3.send('SetReady', { ready: true });
  await c4.send('SetReady', { ready: true });
  await c3.waitForMessage('GameStarted', 10_000);

  await waitForLastMessage(
    c3,
    'RepairState',
    'data && Array.isArray(data.teams) && data.teams[0] === "Idle"',
  );

  await c3.send('ControlSystem', {
    target: 'repair',
    payload: {
      type: 'DispatchRepairTeam',
      data: { team_idx: 0, target: { type: 'Station', data: 'engineering' } },
    },
  });
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

test('Engineering acts when all four connect before selecting', async ({ context }) => {
  const serverPage = await createServerPage(context);
  const hostId = await readHostPeerId(serverPage);

  const c1 = await createTestClient(context, hostId, { name: 'P1' });
  const c2 = await createTestClient(context, hostId, { name: 'P2' });
  const c3 = await createTestClient(context, hostId, { name: 'Eng' });
  const c4 = await createTestClient(context, hostId, { name: 'Sci' });

  await selectAndWait(c1, 'Captain');
  await selectAndWait(c2, 'Helm');
  await selectAndWait(c3, 'Engineering');
  await selectAndWait(c4, 'Science');

  const a3 = await lastAssignment(c3, c3.token);
  expect(a3.data.station_id).toBe('engineering');

  await c1.send('SetReady', { ready: true });
  await c2.send('SetReady', { ready: true });
  await c3.send('SetReady', { ready: true });
  await c4.send('SetReady', { ready: true });
  await c3.waitForMessage('GameStarted', 10_000);

  const boot = await bootHelmPower(c3);
  const moved = otherLevel(boot);
  await setHelmPower(c3, moved);
  await waitForLastMessage(c3, 'PowerState', `data && data.helm === ${moved}`);

  await c1.close();
  await c2.close();
  await c3.close();
  await c4.close();
});

test('Engineering can still act after a mid-game reconnect', async ({ context }) => {
  const { c1, c2, c3, c4, hostId } = await buildFourPlayerCrew(context);
  const powToken = c3.token;

  await c1.send('SetReady', { ready: true });
  await c2.send('SetReady', { ready: true });
  await c3.send('SetReady', { ready: true });
  await c4.send('SetReady', { ready: true });
  await c3.waitForMessage('GameStarted', 10_000);

  const boot = await bootHelmPower(c3);
  const moved = otherLevel(boot);
  await setHelmPower(c3, moved);
  await waitForLastMessage(c3, 'PowerState', `data && data.helm === ${moved}`);

  await c3.close();
  const c3b = await createTestClient(context, hostId, { token: powToken, name: 'Eng' });

  // Wait for any PowerState (AI may have reset helm during disconnect).
  await waitForLastMessage(c3b, 'PowerState', 'data && typeof data.helm === "number"', 10_000);
  // Drive both levels from the reconnected device: whatever the AI did while it
  // was away, the returning holder's taps must still be the ones that land.
  await setHelmPower(c3b, moved);
  await waitForLastMessage(c3b, 'PowerState', `data && data.helm === ${moved}`, 10_000);

  await setHelmPower(c3b, boot);
  await waitForLastMessage(c3b, 'PowerState', `data && data.helm === ${boot}`, 10_000);

  await c1.close();
  await c2.close();
  await c3b.close();
  await c4.close();
});

test('shared session-token orphans the first Engineering device (ghost console)', async ({ context }) => {
  const { c1, c2, c3, c4, hostId } = await buildFourPlayerCrew(context);
  const powToken = c3.token;

  await c1.send('SetReady', { ready: true });
  await c2.send('SetReady', { ready: true });
  await c3.send('SetReady', { ready: true });
  await c4.send('SetReady', { ready: true });
  await c3.waitForMessage('GameStarted', 10_000);

  const boot = await bootHelmPower(c3);
  const moved = otherLevel(boot);

  await setHelmPower(c3, moved);
  await waitForLastMessage(c3, 'PowerState', `data && data.helm === ${moved}`);

  // Record c3's message count before ghost connects
  const preCount = await c3.page.evaluate(() => (window as any).__messages?.length ?? 0);

  const ghostWinner = await createTestClient(context, hostId, { token: powToken, name: 'Eng-2' });
  await waitForLastMessage(ghostWinner, 'PowerState', 'data && typeof data.helm === "number"');

  // ghostWinner sends a change so we can verify c3 (ghost) does NOT receive updates
  await setHelmPower(ghostWinner, boot);
  await waitForLastMessage(ghostWinner, 'PowerState', `data && data.helm === ${boot}`);

  // Small settling window to drain any in-flight SimState (the server tick
  // that might have fired between tokenConns overwrite and this check).
  await c3.page.waitForTimeout(500);

  // Check that c3 did NOT receive the ghostWinner's change back to `boot`.
  // We check for the specific value rather than any PowerState to avoid
  // flakiness from a PowerState that fired just before tokenConns was
  // updated (in-flight via BroadcastChannel past preCount).
  const sawGhostChange = await c3.page.evaluate(
    ({ count, level }: { count: number; level: number }) => {
      const msgs: any[] = (window as any).__messages || [];
      const newMsgs = msgs.slice(count);
      return newMsgs.some((m: any) => m.type === 'PowerState' && m.data?.helm === level);
    },
    { count: preCount, level: boot },
  );
  expect(sawGhostChange).toBe(false);

  await c1.close();
  await c2.close();
  await c3.close();
  await ghostWinner.close();
  await c4.close();
});

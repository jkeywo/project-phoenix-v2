// Regression: the Engineering seat ([Repair, Power]) must honor actions from
// its holder. With the fixed-roster model (#495), Engineering always holds
// exactly [Repair, Power] at the 6P layout — no cascade shrink/grow on join/leave.
//
// Power/Repair both authorize taps against `console_holder(X)` — the same
// function that decides who receives state. So the failure mode we guard
// against is a token/holder desync: the Engineering client believes it holds
// [Repair, Power] (and receives PowerState/RepairState) while the server
// rejects its actions.

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

/**
 * Build a 4-player crew at the fixed 6P layout. No cascade — each player
 * selects their station directly. Returns the four clients; c3 is the
 * Engineering seat under test.
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
  await selectAndWait(c4, 'Sensors');

  return { c1, c2, c3, c4, hostId };
}

test('Engineering seat is assigned exactly [Repair, Power] at 6P layout', async ({ context }) => {
  const { c1, c2, c3, c4 } = await buildFourPlayerCrew(context);

  const a3 = await lastAssignment(c3, c3.token);
  expect(a3.data.station).toBe('Engineering');
  expect([...a3.data.consoles].sort()).toEqual(['Power', 'Repair']);

  await c1.close();
  await c2.close();
  await c3.close();
  await c4.close();
});

test('Engineering player can change power (tap is honored, not just shown)', async ({ context }) => {
  const { c1, c2, c3, c4 } = await buildFourPlayerCrew(context);

  // c1 holds CaptainChair; start the game.
  await c1.send('StartGame');
  await c3.waitForMessage('GameStarted', 5_000);

  // Data delivery: the Engineering seat must receive PowerState (proves it is
  // the recognized Power holder server-side). Default power levels are 2/2/2.
  await waitForLastMessage(c3, 'PowerState', 'data && data.helm === 2');

  // Tap authorization: increasing Helm power must be honored.
  await c3.send('ControlSystem', { target: 'power', payload: { type: 'SetPower', data: { target: 'Helm', level: 3 } } });
  await waitForLastMessage(c3, 'PowerState', 'data && data.helm === 3');

  await c1.close();
  await c2.close();
  await c3.close();
  await c4.close();
});

test('Engineering player can dispatch a repair team (tap is honored)', async ({ context }) => {
  const { c1, c2, c3, c4 } = await buildFourPlayerCrew(context);

  await c1.send('StartGame');
  await c3.waitForMessage('GameStarted', 5_000);

  // Data delivery: the seat receives RepairState with all teams idle.
  await waitForLastMessage(
    c3,
    'RepairState',
    'data && Array.isArray(data.teams) && data.teams[0] === "Idle"',
  );

  // Tap authorization: dispatching team 0 must move it out of Idle.
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

// Realistic ordering: all four players are already connected (sitting in the
// lobby) before anyone selects, so every SelectStation resolves at the 6P
// layout directly — no cascade. The Engineering seat must still be able to act.
test('Engineering acts when all four connect before selecting', async ({ context }) => {
  const serverPage = await createServerPage(context);
  const hostId = await readHostPeerId(serverPage);

  const c1 = await createTestClient(context, hostId, { name: 'P1' });
  const c2 = await createTestClient(context, hostId, { name: 'P2' });
  const c3 = await createTestClient(context, hostId, { name: 'Eng' });
  const c4 = await createTestClient(context, hostId, { name: 'Sci' });

  // All connected (6P layout); now select directly.
  await selectAndWait(c1, 'Captain');
  await selectAndWait(c2, 'Helm');
  await selectAndWait(c3, 'Engineering');
  await selectAndWait(c4, 'Sensors');

  const a3 = await lastAssignment(c3, c3.token);
  expect([...a3.data.consoles].sort()).toEqual(['Power', 'Repair']);

  await c1.send('StartGame');
  await c3.waitForMessage('GameStarted', 5_000);

  await waitForLastMessage(c3, 'PowerState', 'data && data.helm === 2');
  await c3.send('ControlSystem', { target: 'power', payload: { type: 'SetPower', data: { target: 'Helm', level: 3 } } });
  await waitForLastMessage(c3, 'PowerState', 'data && data.helm === 3');

  await c1.close();
  await c2.close();
  await c3.close();
  await c4.close();
});

// Realistic ordering: the Engineering phone drops Wi-Fi mid-game and
// reconnects under the same token (the most likely real-world trigger for a
// host token→connection routing desync). After reconnect the seat must still
// be able to act, not just receive state.
test('Engineering can still act after a mid-game reconnect', async ({ context }) => {
  const { c1, c2, c3, c4, hostId } = await buildFourPlayerCrew(context);
  const engToken = c3.token;

  await c1.send('StartGame');
  await c3.waitForMessage('GameStarted', 5_000);

  // Confirm the seat works before the blip.
  await waitForLastMessage(c3, 'PowerState', 'data && data.helm === 2');
  await c3.send('ControlSystem', { target: 'power', payload: { type: 'SetPower', data: { target: 'Helm', level: 3 } } });
  await waitForLastMessage(c3, 'PowerState', 'data && data.helm === 3');

  // Simulate the Wi-Fi blip: tear down the page (host sees the connection
  // close), then reconnect a fresh page under the SAME token.
  await c3.close();
  const c3b = await createTestClient(context, hostId, { token: engToken, name: 'Eng' });

  // Reconnect must restore Engineering and resume PowerState delivery.
  await waitForLastMessage(c3b, 'PowerState', 'data && typeof data.helm === "number"');

  // The reconnected seat must still be authorized: lower Helm power back down.
  const before = await c3b.page.evaluate(() => {
    const msgs: any[] = (window as any).__messages || [];
    return (msgs.filter((m: any) => m.type === 'PowerState').pop() || {}).data?.helm;
  });
  await c3b.send('ControlSystem', { target: 'power', payload: { type: 'SetPower', data: { target: 'Helm', level: Math.max(1, (before ?? 3) - 1) } } });
  await waitForLastMessage(c3b, 'PowerState', `data && data.helm < ${before}`);

  await c1.close();
  await c2.close();
  await c3b.close();
  await c4.close();
});

// Root-cause reproduction: two devices share the same session-token. The host
// keys connections by token, so the second Identify overwrites the routing
// entry and ORPHANS the first connection. The first device then shows the
// exact reported symptom — its console is frozen (it receives no further
// state) and its taps are accepted server-side but produce no visible feedback
// (the resulting state is routed to the other device). This documents the
// failure mode that the new host-side DUPLICATE TOKEN warning now flags.
test('shared session-token orphans the first Engineering device (ghost console)', async ({ context }) => {
  const { c1, c2, c3, c4, hostId } = await buildFourPlayerCrew(context);
  const engToken = c3.token;

  await c1.send('StartGame');
  await c3.waitForMessage('GameStarted', 5_000);

  // c3 is live: it receives PowerState at the default helm level.
  await waitForLastMessage(c3, 'PowerState', 'data && data.helm === 2');

  // A second device identifies with the SAME token — c3 becomes the ghost.
  const ghostWinner = await createTestClient(context, hostId, { token: engToken, name: 'Eng-2' });
  await waitForLastMessage(ghostWinner, 'PowerState', 'data && typeof data.helm === "number"');

  // The orphaned device taps. The action IS accepted (same token still holds
  // Power), so the *winner* sees helm climb to 3 — but the ghost (c3) never does.
  await c3.send('ControlSystem', { target: 'power', payload: { type: 'SetPower', data: { target: 'Helm', level: 3 } } });
  await waitForLastMessage(ghostWinner, 'PowerState', 'data && data.helm === 3');

  const ghostSawIncrease = await c3.page.evaluate(() => {
    const msgs: any[] = (window as any).__messages || [];
    return msgs.some((m: any) => m.type === 'PowerState' && m.data.helm === 3);
  });
  expect(ghostSawIncrease).toBe(false); // frozen console: taps "did nothing" here

  await c1.close();
  await c2.close();
  await c3.close();
  await ghostWinner.close();
  await c4.close();
});

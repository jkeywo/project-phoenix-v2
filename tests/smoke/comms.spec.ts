// Issue #202 — Comms smoke test: hail contact → respond → objective.
//
// Two clients join a 2-player game (Helm station has CaptainChair+Helm+Comms).
// After game starts, the Helm player hails the first available contact, picks a
// response, and verifies that ObjectiveSummary is delivered to the captain.

import { test, expect, createTestClient, readHostPeerId, waitForWasmReady } from './fixtures';

async function waitForStation(client: { page: import('@playwright/test').Page; token: string }, timeout = 8_000) {
  await client.page.waitForFunction(
    (t) => (window as any).__messages?.some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t,
    ),
    client.token,
    { timeout },
  );
}

test('comms — hail contact, respond, get ObjectiveSummary', async ({ context }) => {
  // ── Boot server ────────────────────────────────────────────────────────────
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  const hostId = await readHostPeerId(serverPage);

  // ── Two clients (6P fixed-roster: Comms = Comms+Navigation, Tactical) ────
  const captain = await createTestClient(context, hostId, { name: 'Captain' });
  const tactical = await createTestClient(context, hostId, { name: 'Tactical' });

  await captain.send('SelectStation', { station: 'Comms' });
  await waitForStation(captain);

  await tactical.send('SelectStation', { station: 'Tactical' });
  await waitForStation(tactical);

  // ── Start game (all players ready -> auto-start) ──────────────────────────
  await captain.send('SetReady', { ready: true });
  await tactical.send('SetReady', { ready: true });
  await captain.waitForMessage('GameStarted', 8_000);

  // ── Wait for initial CommsState (contacts list, sent on first InProgress tick)
  const initialComms = await captain.waitForMessage('CommsState', 8_000) as any;
  const contacts: Array<{ uuid: string; name: string; in_range: boolean }> = initialComms?.data?.contacts ?? [];
  expect(contacts.length, 'at least one contact must be present in initial CommsState').toBeGreaterThan(0);
  const starbase = contacts[0];
  expect(starbase.in_range, 'contact must expose in_range flag').toBeDefined();
  expect(typeof starbase.in_range).toBe('boolean');
  const starbaseUuid = starbase.uuid;

  // ── Hail the contact ──────────────────────────────────────────────────────
  // Clear previously collected CommsState so waitForMessage sees a fresh one.
  await captain.page.evaluate(() => {
    (window as any).__messages = (window as any).__messages.filter(
      (m: any) => m.type !== 'CommsState',
    );
  });

  await captain.send('ControlSystem', { target: 'comms', payload: { type: 'Hail', data: { target_uuid: starbaseUuid } } });

  const hailComms = await captain.waitForMessage('CommsState', 8_000) as any;
  const messages: Array<{ id: string; body: string; responses: string[]; sender_in_range: boolean }> =
    hailComms?.data?.messages ?? [];
  expect(messages.length).toBeGreaterThan(0);
  const msg = messages[0];
  expect(msg.responses.length).toBeGreaterThan(0);
  expect(msg.sender_in_range, 'message must expose sender_in_range flag').toBeDefined();
  expect(typeof msg.sender_in_range).toBe('boolean');

  // ── Respond ────────────────────────────────────────────────────────────────
  await captain.page.evaluate(() => {
    (window as any).__messages = (window as any).__messages.filter(
      (m: any) => m.type !== 'CommsState' && m.type !== 'ObjectiveSummary',
    );
  });

  await captain.send('ControlSystem', { target: 'comms', payload: { type: 'RespondToMessage', data: { message_id: msg.id, response_index: 0 } } });

  // CommsState must reflect the selected response.
  const respondComms = await captain.waitForMessage('CommsState', 8_000) as any;
  const updatedMsg = (respondComms?.data?.messages ?? []).find(
    (m: any) => m.id === msg.id,
  );
  expect(updatedMsg, 'original message must still be in inbox after response').toBeTruthy();
  expect(updatedMsg.selected_response).toBe(0);

  // Captain receives ObjectiveSummary with one new objective.
  const objSummary = await captain.waitForMessage('ObjectiveSummary', 8_000) as any;
  const objectives: Array<{ id: string; text: string }> = objSummary?.data?.objectives ?? [];
  expect(objectives.length).toBeGreaterThan(0);

  await captain.close();
  await tactical.close();
});

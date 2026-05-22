// Issue #202 — Comms smoke test: Starbase Alpha hail → respond → objective.
//
// Two clients join a 2-player game (Helm station has CaptainChair+Helm+Comms).
// After game starts, the Helm player hails Starbase Alpha, picks a response,
// and verifies that ObjectiveSummary is delivered to the captain.

import { test, expect, createTestClient, readHostPeerId } from './fixtures';

async function waitForStation(client: { page: import('@playwright/test').Page; token: string }, timeout = 8_000) {
  await client.page.waitForFunction(
    (t) => (window as any).__messages?.some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t,
    ),
    client.token,
    { timeout },
  );
}

test('comms — hail Starbase Alpha, respond, get ObjectiveSummary', async ({ context }) => {
  // ── Boot server ────────────────────────────────────────────────────────────
  const serverPage = await context.newPage();
  await serverPage.goto('/');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 20_000 });

  const hostId = await readHostPeerId(serverPage);

  // ── Two clients (2P layout: Helm = CaptainChair+Helm+Comms, Tactical) ────
  const captain = await createTestClient(context, hostId, { name: 'Captain' });
  const tactical = await createTestClient(context, hostId, { name: 'Tactical' });

  await captain.send('SelectStation', { station: 'Helm' });
  await waitForStation(captain);

  await tactical.send('SelectStation', { station: 'Tactical' });
  await waitForStation(tactical);

  // ── Start game ─────────────────────────────────────────────────────────────
  await captain.send('StartGame');
  await captain.waitForMessage('GameStarted', 8_000);

  // ── Wait for initial CommsState (contacts list, sent on first InProgress tick)
  const initialComms = await captain.waitForMessage('CommsState', 8_000) as any;
  const contacts: Array<{ uuid: string; name: string }> = initialComms?.data?.contacts ?? [];
  const starbase = contacts.find((c) => c.name === 'Starbase Alpha');
  expect(starbase, 'Starbase Alpha contact must be present in initial CommsState').toBeTruthy();
  expect(starbase!.in_range, 'contact must expose in_range flag').toBeDefined();
  expect(typeof starbase!.in_range).toBe('boolean');
  const starbaseUuid = starbase!.uuid;

  // ── Hail Starbase Alpha ────────────────────────────────────────────────────
  // Clear previously collected CommsState so waitForMessage sees a fresh one.
  await captain.page.evaluate(() => {
    (window as any).__messages = (window as any).__messages.filter(
      (m: any) => m.type !== 'CommsState',
    );
  });

  await captain.send('Hail', { target_uuid: starbaseUuid });

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

  await captain.send('RespondToMessage', { message_id: msg.id, response_index: 0 });

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

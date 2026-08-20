// Issue #1104 — Smoke tests: the AFK (Away From Keyboard) presence flag that
// delegates a retained Station while its player is away. Mirrors
// tests/smoke/spectator.spec.js's station protocol shape (#1105) and the
// mid-game relocation shape (#1099).
//
// Covers, end to end over the real datachannel: enter AFK while KEEPING the seat
// and reconnect identity (AC1), the held Station delegating to AI/Backfill
// (AC2), leaving AFK restoring the prior rating WITHOUT vacating the seat or
// re-sending a StationAssigned — so no console-focus steal (AC4), other players
// receiving the presence delta as a bare boolean (AC5), and reconnect-stays-AFK
// (AC5).

import { test, expect } from './fixtures';
import { readHostPeerId, createTestClient, createServerPage } from './fixtures';

async function bootServer(context) {
  const serverPage = await createServerPage(context);
  return readHostPeerId(serverPage);
}

/** Wait until this token has received a StationAssigned holding a station. */
async function waitForStationAssigned(client) {
  await client.page.waitForFunction(
    (t) => window.__messages?.some(
      (m) => m.type === 'StationAssigned' && m.data.token === t && m.data.station_id != null,
    ),
    client.token,
    { timeout: 5_000 },
  );
}

/** Enter AFK and wait for this token's own AfkChanged{true}. */
async function enterAfk(client) {
  await client.send('SetAfk', { afk: true });
  await client.page.waitForFunction(
    (t) => window.__messages?.some(
      (m) => m.type === 'AfkChanged' && m.data.token === t && m.data.afk === true,
    ),
    client.token,
    { timeout: 5_000 },
  );
}

/** Wait for a RatingChanged whose rating_name satisfies `pred`, after a marker
 *  index into __messages (so we only see deltas produced AFTER an action). */
async function waitForRatingChangedAfter(client, fromIndex, isBackfill) {
  await client.page.waitForFunction(
    ([from, backfill]) => (window.__messages || [])
      .slice(from)
      .some((m) => m.type === 'RatingChanged'
        && (backfill ? m.data.rating_name === 'Backfill' : m.data.rating_name !== 'Backfill')),
    [fromIndex, isBackfill],
    { timeout: 15_000 },
  );
}

const msgCount = (client) => client.page.evaluate(() => (window.__messages || []).length);

test('AC1 — a station holder enters AFK and keeps the seat', async ({ context }) => {
  const hostId = await bootServer(context);

  const c = await createTestClient(context, hostId, { name: 'Helm' });
  await c.send('SelectStation', { station: 'Helm' });
  await waitForStationAssigned(c);

  await c.send('SetAfk', { afk: true });
  const changed = await c.waitForMessage('AfkChanged');
  expect(changed.data.token).toBe(c.token);
  expect(changed.data.afk).toBe(true);

  // AFK RETAINS the seat: no StationAssigned{None} is ever emitted for this token.
  const vacated = await c.page.evaluate(
    (t) => (window.__messages || []).some(
      (m) => m.type === 'StationAssigned' && m.data.token === t && m.data.station === null,
    ),
    c.token,
  );
  expect(vacated).toBe(false);

  await c.close();
});

test('AC5 — the AFK delta carries only the boolean and reaches other players', async ({ context }) => {
  const hostId = await bootServer(context);

  const crew = await createTestClient(context, hostId, { name: 'Helm' });
  const other = await createTestClient(context, hostId, { name: 'Onlooker' });

  await crew.send('SelectStation', { station: 'Helm' });
  await waitForStationAssigned(crew);

  await enterAfk(crew);

  // The OTHER player sees the presence effect — and it is a bare boolean: the
  // AfkChanged payload has exactly `token` and `afk`, no accessibility detail.
  const seen = await other.waitForMessage('AfkChanged');
  expect(seen.data.token).toBe(crew.token);
  expect(seen.data.afk).toBe(true);
  expect(Object.keys(seen.data).sort()).toEqual(['afk', 'token']);

  await crew.close();
  await other.close();
});

test('AC2/AC4 — in-game AFK delegates the held station to Backfill, and leaving restores it without a focus steal', async ({ context }) => {
  const hostId = await bootServer(context);

  const crew = await createTestClient(context, hostId, { name: 'Helm' });
  await crew.send('SelectStation', { station: 'Helm' });
  await waitForStationAssigned(crew);

  // Start the mission: the lone crew member readying is enough.
  await crew.send('SetReady', { ready: true });
  await crew.waitForMessage('GameStarted', 15_000);

  // AC2: entering AFK delegates the held station — a RatingChanged{Backfill}
  // for the seat arrives (every owned system moves to AI control).
  const beforeEnter = await msgCount(crew);
  await crew.send('SetAfk', { afk: true });
  await waitForRatingChangedAfter(crew, beforeEnter, true);

  // AC4: leaving AFK restores the prior (non-Backfill) rating…
  const beforeLeave = await msgCount(crew);
  await crew.send('SetAfk', { afk: false });
  await waitForRatingChangedAfter(crew, beforeLeave, false);
  await crew.page.waitForFunction(
    ([from, t]) => (window.__messages || [])
      .slice(from)
      .some((m) => m.type === 'AfkChanged' && m.data.token === t && m.data.afk === false),
    [beforeLeave, crew.token],
    { timeout: 5_000 },
  );

  // …WITHOUT re-sending a StationAssigned — the seat was never vacated, so the
  // client keeps its current console (no tab-focus steal).
  const reassignedOnReturn = await crew.page.evaluate(
    ([from, t]) => (window.__messages || [])
      .slice(from)
      .some((m) => m.type === 'StationAssigned' && m.data.token === t),
    [beforeEnter, crew.token],
  );
  expect(reassignedOnReturn).toBe(false);

  await crew.close();
});

test('AC1/AC5 — an AFK holder that reconnects stays AFK and keeps its seat', async ({ context }) => {
  const hostId = await bootServer(context);
  const TOKEN = 'afk-reconnect';

  const c = await createTestClient(context, hostId, { token: TOKEN, name: 'Helm' });
  await c.send('SelectStation', { station: 'Helm' });
  await waitForStationAssigned(c);
  await enterAfk(c);

  // Simulate a browser refresh: drop and reconnect with the same token.
  await c.close();
  const c2 = await createTestClient(context, hostId, { token: TOKEN, name: 'Helm' });

  const welcome = await c2.waitForMessage('Welcome');
  const me = (welcome?.data?.state?.players ?? []).find((p) => p.token === TOKEN);
  expect(me?.afk).toBe(true);
  // The seat and reconnect identity are retained across the drop.
  const held = me?.station ?? (me?.station_id ?? null);
  expect(held).not.toBeNull();

  await c2.close();
});

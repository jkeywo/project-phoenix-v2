// Issue #1105 — Smoke tests: the explicit Spectator role and its crew-public
// summary surface. Mirrors tests/smoke/lobby.spec.js's station protocol shape.
//
// Covers, end to end over the real datachannel: join-as-spectator (AC1),
// readiness exclusion (AC2 — a seated player readies and the game starts while
// a spectator sits), simulation-command refusal + retained connection (AC3),
// the crew-public summary surface being fed (AC4), and reconnect-stays-spectator
// (AC5).

import { test, expect } from './fixtures';
import { readHostPeerId, createTestClient, createServerPage } from './fixtures';

async function bootServer(context) {
  const serverPage = await createServerPage(context);
  return readHostPeerId(serverPage);
}

/** Wait until this token has received a StationAssigned for itself. */
async function waitForStationAssigned(client) {
  await client.page.waitForFunction(
    (t) => window.__messages?.some((m) => m.type === 'StationAssigned' && m.data.token === t),
    client.token,
    { timeout: 5_000 },
  );
}

/** Wait until this token has received a SpectatorChanged for itself. */
async function becomeSpectator(client) {
  await client.send('SetSpectator', { spectator: true });
  await client.page.waitForFunction(
    (t) => window.__messages?.some(
      (m) => m.type === 'SpectatorChanged' && m.data.token === t && m.data.spectator === true,
    ),
    client.token,
    { timeout: 5_000 },
  );
}

test('AC1 — join explicitly as a spectator (SpectatorChanged broadcast)', async ({ context }) => {
  const hostId = await bootServer(context);

  const spec = await createTestClient(context, hostId, { name: 'Watcher' });
  await spec.send('SetSpectator', { spectator: true });

  const changed = await spec.waitForMessage('SpectatorChanged');
  expect(changed.data.token).toBe(spec.token);
  expect(changed.data.spectator).toBe(true);

  await spec.close();
});

test('AC1 — becoming a spectator vacates any held station', async ({ context }) => {
  const hostId = await bootServer(context);

  const c = await createTestClient(context, hostId, { name: 'Switcher' });
  await c.send('SelectStation', { station: 'Helm' });
  await waitForStationAssigned(c);

  await c.send('SetSpectator', { spectator: true });
  await c.waitForMessage('SpectatorChanged');

  // The seat is vacated: a StationAssigned{None} for this token must arrive.
  const vacated = await c.page.evaluate(
    (t) => (window.__messages || []).some(
      (m) => m.type === 'StationAssigned' && m.data.token === t && m.data.station === null,
    ),
    c.token,
  );
  expect(vacated).toBe(true);

  await c.close();
});

test('AC2 — a seated player readies and the game starts while a spectator sits', async ({ context }) => {
  const hostId = await bootServer(context);

  const crew = await createTestClient(context, hostId, { name: 'Helm' });
  const spec = await createTestClient(context, hostId, { name: 'Watcher' });

  await becomeSpectator(spec);

  await crew.send('SelectStation', { station: 'Helm' });
  await waitForStationAssigned(crew);

  // The spectator NEVER readies. The lone crew member readying is enough to
  // start — a sitting spectator neither counts toward readiness nor delays it.
  await crew.send('SetReady', { ready: true });

  await crew.waitForMessage('GameStarted', 15_000);
  // GameStarted is Audience::All, so the spectator sees it too.
  await spec.waitForMessage('GameStarted', 15_000);

  await crew.close();
  await spec.close();
});

test('AC3 — a spectator cannot start the game and its SetReady is a no-op', async ({ context }) => {
  const hostId = await bootServer(context);

  const spec = await createTestClient(context, hostId, { name: 'Watcher' });
  await becomeSpectator(spec);

  // A spectator's SetReady must neither flip its flag nor start a countdown —
  // a spectator-only lobby never auto-starts.
  await spec.send('SetReady', { ready: true });
  await spec.page.waitForTimeout(750);

  const started = await spec.lastMessage('GameStarted');
  expect(started).toBeNull();

  const readied = await spec.page.evaluate(
    (t) => (window.__messages || []).some(
      (m) => m.type === 'ReadyChanged' && m.data.token === t && m.data.ready === true,
    ),
    spec.token,
  );
  expect(readied).toBe(false);

  await spec.close();
});

test('AC3/AC4 — an in-game spectator keeps its connection and is fed crew-public state', async ({ context }) => {
  const hostId = await bootServer(context);

  const crew = await createTestClient(context, hostId, { name: 'Helm' });
  const spec = await createTestClient(context, hostId, { name: 'Watcher' });

  await becomeSpectator(spec);
  await crew.send('SelectStation', { station: 'Helm' });
  await waitForStationAssigned(crew);
  await crew.send('SetReady', { ready: true });
  await spec.waitForMessage('GameStarted', 15_000);

  // The spectator issues a simulation command. Admission drops it silently; the
  // ordinary connection is retained and the crew-public surface keeps updating.
  await spec.send('ControlSystem', {
    target: 'helm-thrust',
    payload: { type: 'SetThrust', value: 1.0 },
  });

  // The crew-public SimState broadcast (Audience::All) still reaches the
  // spectator — the summary surface is fed and the link is alive.
  await spec.waitForMessage('SimState', 15_000);

  await crew.close();
  await spec.close();
});

test('AC5 — a spectator that reconnects stays a spectator and out of readiness', async ({ context }) => {
  const hostId = await bootServer(context);
  const TOKEN = 'spec-reconnect';

  const spec = await createTestClient(context, hostId, { token: TOKEN, name: 'Watcher' });
  await becomeSpectator(spec);

  // Simulate a browser refresh: drop and reconnect with the same token.
  await spec.close();
  const spec2 = await createTestClient(context, hostId, { token: TOKEN, name: 'Watcher' });

  const welcome = await spec2.waitForMessage('Welcome');
  const me = (welcome?.data?.state?.players ?? []).find((p) => p.token === TOKEN);
  expect(me?.spectator).toBe(true);
  expect(me?.station ?? null).toBeNull();

  await spec2.close();
});

// Issue #1112 AC1 — a browser host accepts MULTIPLE browser crew clients
// through Phoenix signalling, with no PeerJS anywhere in the picture.
//
// Other specs in this directory run two, three and four clients while testing
// something else; this one exists to make the claim itself the subject, because
// "several phones on one code" is the property a rendezvous cutover is most
// likely to break quietly. One host issues one code; four phones resolve that
// same code through the service, each opens its own direct connection, and the
// host keeps their identities, seats and targeted traffic apart.

import { test, expect, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';

const CREW = [
  { name: 'Ada', station: 'Captain' },
  { name: 'Bo', station: 'Helm' },
  { name: 'Cy', station: 'Tactical' },
  { name: 'Dee', station: 'Engineering' },
];

test('four phones join one host on one code and hold four different seats', { tag: '@core' }, async ({ context }) => {
  test.setTimeout(90_000);

  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  // ONE code, issued once. Every phone below resolves this same string.
  const joinCode = await readHostPeerId(serverPage);
  expect(joinCode).toContain('_');

  const clients = [];
  for (const who of CREW) {
    clients.push(await createTestClient(context, joinCode, {
      token: `tok-${who.name.toLowerCase()}`,
      name: who.name,
    }));
  }

  // The host's own per-token bookkeeping: four reliable connections and four
  // lossy ones, keyed by four distinct session tokens. This is what
  // routeOutbound routes on, so a collision here would silently deliver one
  // phone's targeted messages to another.
  const hostState = await serverPage.evaluate((tokens) => {
    // eslint-disable-next-line no-eval
    const conns = (0, eval)('tokenConns');
    // eslint-disable-next-line no-eval
    const snaps = (0, eval)('tokenSnapshotConns');
    return {
      reliable: tokens.filter((t) => conns.has(t)).length,
      lossy: tokens.filter((t) => snaps.has(t)).length,
      total: conns.size,
    };
  }, clients.map((c) => c.token));
  expect(hostState).toEqual({ reliable: 4, lossy: 4, total: 4 });

  // Every phone sees the whole crew — the lobby roster crossed four separate
  // connections, not one broadcast that happened to reach the first.
  for (const client of clients) {
    await client.page.waitForFunction(
      () => {
        const welcome = window.__messages.filter((m) => m.type === 'Welcome').pop();
        const joined = window.__messages.filter((m) => m.type === 'PlayerJoined').length;
        return !!welcome && (welcome.data?.state?.players?.length ?? 0) + joined >= 4;
      },
      undefined,
      { timeout: 15_000 },
    );
  }

  // Four different seats, each assigned to the token that asked for it.
  for (let i = 0; i < CREW.length; i += 1) {
    await clients[i].send('SelectStation', { station: CREW[i].station });
  }
  for (let i = 0; i < CREW.length; i += 1) {
    const client = clients[i];
    await client.page.waitForFunction(
      (token) => window.__messages.some(
        (m) => m.type === 'StationAssigned' && m.data.token === token && m.data.station,
      ),
      client.token,
      { timeout: 15_000 },
    );
    const assigned = await client.page.evaluate(
      (token) => window.__messages
        .filter((m) => m.type === 'StationAssigned' && m.data.token === token)
        .pop().data.station,
      client.token,
    );
    expect(assigned.toLowerCase()).toBe(CREW[i].station.toLowerCase());
  }

  // Collective auto-start, then snapshot-class traffic to all four.
  for (const client of clients) await client.send('SetReady', { ready: true });
  for (const client of clients) await client.waitForMessage('GameStarted', 20_000);
  for (const client of clients) {
    const sim = await client.waitForMessage('SimState', 10_000);
    expect(Array.isArray(sim.data.snapshot?.entity_states)).toBe(true);
    // …and it took the lossy channel this phone negotiated for itself.
    await client.page.waitForFunction(
      () => window.__snapshotMessages.some((m) => m.type === 'SimState'),
      undefined,
      { timeout: 10_000 },
    );
  }

  // One phone leaving takes only its own seat with it.
  await clients[3].close();
  await serverPage.waitForFunction(
    // eslint-disable-next-line no-eval
    (token) => !(0, eval)('tokenConns').has(token),
    clients[3].token,
    { timeout: 10_000 },
  );
  expect(await serverPage.evaluate(() => (0, eval)('tokenConns').size)).toBe(3);

  for (const client of clients.slice(0, 3)) await client.close();
});

// The reliable/lossy delivery split, over the Phoenix transport (issue #1112,
// carrying forward the behaviour the PeerJS-era version of this spec pinned).
//
// Three claims, unchanged from that version — only the mechanism moved. A test
// client used to reach into `conn.peerConnection.createDataChannel('snapshot')`
// by hand; it now asks `createTestClient` for the lossy channel (or not),
// because negotiating both channels is what the shipped client does and the
// fixture is where that lives.
//
//   1. snapshot-class messages ride the lossy channel when it is up,
//   2. they fall back to the reliable channel for a client that has none,
//   3. Welcome and commands use the reliable channel either way.

import { test, expect, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';

/** Labels of the DataChannels this page holds, deduplicated. */
function channelLabels(page) {
  return page.evaluate(() =>
    [...new Set(Object.keys(window.__transportShim.dataChannels()).map((k) => k.split('@')[0]))].sort());
}

/** The lossy channel's negotiated parameters, as the far end would see them. */
function lossyParams(page) {
  return page.evaluate(() => {
    const entry = Object.entries(window.__transportShim.dataChannels())
      .find(([key]) => key.startsWith('snapshot@'));
    return entry ? { ordered: entry[1].ordered, maxRetransmits: entry[1].maxRetransmits } : null;
  });
}

async function bootHost(context) {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);
  return { serverPage, hostId: await readHostPeerId(serverPage) };
}

test('SimState arrives via the lossy channel when it is available', async ({ context }) => {
  const { serverPage, hostId } = await bootHost(context);
  const client = await createTestClient(context, hostId, { name: 'Tester' });

  // The lossy channel really is lossy — an unordered, non-retransmitting one
  // negotiated alongside the reliable channel, not the reliable channel under
  // another name.
  expect(await channelLabels(client.page)).toEqual(['reliable', 'snapshot']);
  expect(await lossyParams(client.page)).toEqual({ ordered: false, maxRetransmits: 0 });
  // The host holds its own end of both, which is what routeOutbound routes on.
  expect(await channelLabels(serverPage)).toEqual(['reliable', 'snapshot']);

  await client.send('SelectStation', { station: 'Helm' });
  await client.send('SetReady', { ready: true });
  await client.waitForMessage('GameStarted', 10_000);

  const simState = await client.waitForMessage('SimState', 3_000);
  expect(Array.isArray(simState.data.snapshot?.entity_states)).toBe(true);

  // …and it arrived ON the lossy channel, not merely while one existed. The
  // test client logs each inbound message under the channel it came in on.
  await client.page.waitForFunction(
    () => window.__snapshotMessages?.some((m) => m.type === 'SimState'),
    undefined,
    { timeout: 5_000 },
  );
  expect(await client.page.evaluate(
    () => window.__reliableMessages.some((m) => m.type === 'SimState'),
  )).toBe(false);

  const bb = await client.waitForMessage('BlackboardUpdate', 3_000);
  expect(Array.isArray(bb.data.updates)).toBe(true);

  await client.close();
});

test('SimState falls back to the reliable channel for a client with no lossy one', async ({ context }) => {
  const { hostId } = await bootHost(context);

  // A client that never negotiated the lossy channel. The host's per-token
  // fallback (gui/host-peer-routing.js) is what has to notice.
  const client = await createTestClient(context, hostId, {
    name: 'FallbackTester',
    snapshot: false,
  });
  expect(await channelLabels(client.page)).toEqual(['reliable']);

  await client.send('SelectStation', { station: 'Helm' });
  await client.send('SetReady', { ready: true });
  await client.waitForMessage('GameStarted', 10_000);

  const simState = await client.waitForMessage('SimState', 3_000);
  expect(Array.isArray(simState.data.snapshot?.entity_states)).toBe(true);
  expect(await client.page.evaluate(
    () => window.__reliableMessages.some((m) => m.type === 'SimState'),
  )).toBe(true);

  // A second one, so this proves a sustained fallback rather than one message
  // that happened to be in flight.
  const simState2 = await client.waitForMessage('SimState', 4_000);
  expect(Array.isArray(simState2.data.snapshot?.entity_states)).toBe(true);

  await client.close();
});

test('the fallback is per token, not global', async ({ context }) => {
  // Two phones on one host, one with a lossy channel and one without. The
  // downgrade must follow the client that needs it and leave the other alone —
  // otherwise one old phone drags the whole bridge onto the reliable channel.
  const { serverPage, hostId } = await bootHost(context);
  const lossy = await createTestClient(context, hostId, { name: 'Lossy' });
  const plain = await createTestClient(context, hostId, { name: 'Plain', snapshot: false });

  await lossy.send('SelectStation', { station: 'Helm' });
  await plain.send('SelectStation', { station: 'Captain' });
  // Auto-start is collective: both connected phones have to be ready.
  await lossy.send('SetReady', { ready: true });
  await plain.send('SetReady', { ready: true });
  await lossy.waitForMessage('GameStarted', 15_000);

  expect(await lossy.waitForMessage('SimState', 4_000)).toBeTruthy();
  expect(await plain.waitForMessage('SimState', 4_000)).toBeTruthy();

  // The host holds one lossy channel — the one phone that opened it.
  const hostChannels = await serverPage.evaluate(() =>
    Object.keys(window.__transportShim.dataChannels()).map((k) => k.split('@')[0]).sort());
  expect(hostChannels.filter((l) => l === 'snapshot')).toHaveLength(1);
  expect(hostChannels.filter((l) => l === 'reliable')).toHaveLength(2);

  await lossy.close();
  await plain.close();
});

test('Welcome and commands use the reliable channel regardless of lossy availability', async ({ context }) => {
  const { hostId } = await bootHost(context);
  const client = await createTestClient(context, hostId, { name: 'ReliableTester' });

  // Welcome already arrived during createTestClient — verify it, and verify it
  // came in on the RELIABLE channel. A Welcome that rode the lossy path could
  // be dropped, and a phone that never receives one never gets a seat.
  const welcome = await client.lastMessage('Welcome');
  expect(welcome).not.toBeNull();
  expect(welcome.data?.ship_stations).toBeDefined();
  expect(await client.page.evaluate(() => ({
    reliable: window.__reliableMessages.some((m) => m.type === 'Welcome'),
    lossy: window.__snapshotMessages.some((m) => m.type === 'Welcome'),
  }))).toEqual({ reliable: true, lossy: false });

  await client.close();
});

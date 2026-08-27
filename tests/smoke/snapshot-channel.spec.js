import { test, expect, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';

/** Record messages received by the reliable PeerJS DataConnection. */
async function setupReliableTrace(client) {
  await client.page.evaluate(() => {
    const conn = window.__conn;
    window.__reliableMessages = [];
    if (!conn) return;
    conn.on('data', (raw) => {
      try { window.__reliableMessages.push(JSON.parse(raw)); } catch { /* ignore */ }
    });
  });
}

/**
 * Helper: set up a snapshot DataChannel on the test client page.
 * Must be called after createTestClient returns (__conn is the reliable DataConnection).
 */
async function setupSnapshotChannel(client) {
  await setupReliableTrace(client);
  await client.page.evaluate(() => {
    const conn = window.__conn;
    if (!conn || !conn.peerConnection) return;
    const snapChan = conn.peerConnection.createDataChannel('snapshot', { ordered: false, maxRetransmits: 0 });
    window.__snapChan = snapChan;
    window.__snapshotMessages = [];
    window.__snapshotReady = false;
    snapChan.onopen = () => { window.__snapshotReady = true; };
    snapChan.onclose = () => { window.__snapshotReady = false; };
    snapChan.onmessage = (event) => {
      try {
        const message = JSON.parse(event.data);
        window.__snapshotMessages.push(message);
        window.__messages.push(message);
      } catch { /* ignore */ }
    };
  });
  // Wait for the shim to propagate the datachannel creation and open
  await client.page.waitForFunction(() => window.__snapshotReady === true, { timeout: 2_000 });
}

/**
 * Connect a client, open the snapshot channel, and deliberately withhold
 * Identify until the host has observed that channel as open. This reproduces
 * the production ConnectionManager ordering and makes the cross-stream race
 * deterministic instead of depending on BroadcastChannel scheduling.
 */
async function createSnapshotFirstClient(context, serverPage, hostId) {
  const token = 'snapshot-first-' + Math.random().toString(16).slice(2, 10);
  const name = 'SnapshotFirstTester';
  const page = await context.newPage();
  const routeKey = Math.random().toString(16).slice(2, 10);

  await page.route(`**/blank-${routeKey}`, (route) =>
    route.fulfill({ contentType: 'text/html', body: '<!DOCTYPE html><html><body></body></html>' }),
  );
  await page.goto(`http://localhost:3000/blank-${routeKey}`);

  await page.evaluate(
    ({ hostId }) => new Promise((resolve, reject) => {
      window.__messages = [];
      window.__reliableMessages = [];
      window.__snapshotMessages = [];
      window.__snapshotReady = false;
      const peer = new window.Peer();
      window.__peer = peer;
      const timeout = setTimeout(() => reject(new Error('snapshot channel open timeout')), 5_000);

      peer.on('open', () => {
        const conn = peer.connect(hostId);
        window.__conn = conn;
        conn.on('open', () => {
          const pc = conn.peerConnection;
          if (!pc) {
            clearTimeout(timeout);
            reject(new Error('reliable connection has no peerConnection'));
            return;
          }
          const snapChan = pc.createDataChannel('snapshot', { ordered: false, maxRetransmits: 0 });
          window.__snapChan = snapChan;
          snapChan.onopen = () => {
            window.__snapshotReady = true;
            clearTimeout(timeout);
            resolve();
          };
          snapChan.onclose = () => { window.__snapshotReady = false; };
          snapChan.onmessage = (event) => {
            try {
              const message = JSON.parse(event.data);
              window.__snapshotMessages.push(message);
              window.__messages.push(message);
            } catch { /* ignore */ }
          };
        });
        conn.on('data', (raw) => {
          try {
            const message = JSON.parse(raw);
            window.__reliableMessages.push(message);
            window.__messages.push(message);
          } catch { /* ignore */ }
        });
      });
    }),
    { hostId },
  );

  // Local open is not enough: wait until the host-side counterpart is open,
  // while the host still has no token with which to promote it.
  await serverPage.waitForFunction(
    () => Object.entries(window.__peerjsShim._dataChannels()).some(
      ([key, channel]) => key.endsWith(':snapshot') && channel.readyState === 'open',
    ),
    undefined,
    { timeout: 2_000 },
  );

  // PeerJS itself owns the RTCPeerConnection.ondatachannel callback that
  // initialises the incoming reliable DataConnection. The host's early
  // snapshot listener must delegate that non-snapshot event instead of
  // clobbering it; the shim deliberately withholds host-side `open` unless the
  // owned callback ran.
  const clientPeerId = await page.evaluate(() => window.__peer.id);
  const reliableChannels = await serverPage.evaluate(
    () => window.__peerjsShim._reliableChannels(),
  );
  expect(reliableChannels[`${hostId}->${clientPeerId}`]).toBe(true);

  await page.evaluate(
    ({ token, name }) => {
      window.__conn.send(JSON.stringify({ type: 'Identify', data: { token, name } }));
    },
    { token, name },
  );
  await page.waitForFunction(
    () => window.__reliableMessages.some((message) => message.type === 'Welcome'),
    undefined,
    { timeout: 15_000 },
  );

  return {
    page,
    token,
    async send(type, data) {
      await page.evaluate(
        ({ type, data }) => {
          const message = data !== undefined ? { type, data } : { type };
          window.__conn.send(JSON.stringify(message));
        },
        { type, data },
      );
    },
    async close() { await page.close(); },
  };
}

test('SimState arrives via snapshot channel when available', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);

  const client = await createTestClient(context, hostId, { name: 'Tester' });

  // Set up snapshot DataChannel so server routes snapshot traffic through it
  await setupSnapshotChannel(client);

  await client.send('SelectStation', { station: 'Helm' });
  await client.send('SetReady', { ready: true });
  await client.waitForMessage('GameStarted', 10_000);

  // A reliable classification regression would still put SimState in the
  // aggregate message list, so wait on the channel-specific trace itself.
  await client.page.waitForFunction(
    () => window.__snapshotMessages?.some((m) => m.type === 'SimState'),
    undefined,
    { timeout: 3_000 },
  );
  const simState = await client.page.evaluate(
    () => window.__snapshotMessages.find((m) => m.type === 'SimState'),
  );
  expect(Array.isArray(simState.data.snapshot?.entity_states)).toBe(true);

  // Verify the snapshot channel was used (shim registers it per the label)
  const channels = await client.page.evaluate(() => window.__peerjsShim._dataChannels());
  const hasSnapshot = Object.keys(channels).some(k => k.includes(':snapshot'));
  expect(hasSnapshot).toBe(true);

  // BlackboardUpdate is snapshot traffic too, not merely a message that can
  // happen to arrive over the reliable fallback.
  await client.page.waitForFunction(
    () => window.__snapshotMessages?.some((m) => m.type === 'BlackboardUpdate'),
    undefined,
    { timeout: 3_000 },
  );
  const bb = await client.page.evaluate(
    () => window.__snapshotMessages.find((m) => m.type === 'BlackboardUpdate'),
  );
  expect(Array.isArray(bb.data.updates)).toBe(true);

  const routes = await client.page.evaluate(() => ({
    snapshot: (window.__snapshotMessages || []).map((m) => m.type),
    reliable: (window.__reliableMessages || []).map((m) => m.type),
  }));
  expect(routes.snapshot).toContain('SimState');
  expect(routes.snapshot).toContain('BlackboardUpdate');
  expect(routes.reliable).toContain('GameStarted');
  expect(routes.snapshot).not.toContain('GameStarted');

  await client.close();
});

test('snapshot channel opened before Identify is promoted when the token arrives', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);

  const client = await createSnapshotFirstClient(context, serverPage, hostId);
  await client.send('SelectStation', { station: 'Helm' });
  await client.send('SetReady', { ready: true });

  await client.page.waitForFunction(
    () => window.__reliableMessages.some((message) => message.type === 'GameStarted'),
    undefined,
    { timeout: 10_000 },
  );
  await client.page.waitForFunction(
    () => window.__snapshotMessages.some((message) => message.type === 'SimState'),
    undefined,
    { timeout: 10_000 },
  );
  const routes = await client.page.evaluate(() => ({
    snapshot: window.__snapshotMessages.map((message) => message.type),
    reliable: window.__reliableMessages.map((message) => message.type),
  }));
  expect(routes.snapshot).toContain('SimState');
  expect(routes.reliable).not.toContain('SimState');
  expect(routes.reliable).toContain('GameStarted');

  await client.close();
});

test('SimState falls back to reliable channel when snapshot channel is unavailable', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);

  // Create a client that does NOT set up a snapshot channel
  const client = await createTestClient(context, hostId, { name: 'FallbackTester' });
  await setupReliableTrace(client);

  await client.send('SelectStation', { station: 'Helm' });
  await client.send('SetReady', { ready: true });
  await client.waitForMessage('GameStarted', 10_000);

  // SimState should still arrive via the reliable channel fallback. Observe
  // that channel directly so the assertion cannot be satisfied by an
  // aggregate message list.
  await client.page.waitForFunction(
    () => window.__reliableMessages?.some((m) => m.type === 'SimState'),
    undefined,
    { timeout: 3_000 },
  );
  const simState = await client.page.evaluate(
    () => window.__reliableMessages.find((m) => m.type === 'SimState'),
  );
  expect(Array.isArray(simState.data.snapshot?.entity_states)).toBe(true);

  // Verify no snapshot channel was registered
  const channels = await client.page.evaluate(() => window.__peerjsShim._dataChannels());
  const hasSnapshot = Object.keys(channels).some(k => k.includes(':snapshot'));
  expect(hasSnapshot).toBe(false);

  // Verify a second reliable SimState still arrives (fallback is sustained).
  await client.page.waitForFunction(
    () => window.__reliableMessages?.filter((m) => m.type === 'SimState').length >= 2,
    undefined,
    { timeout: 4_000 },
  );

  await client.close();
});

test('post-setup lobby traffic remains reliable when snapshot channel is available', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);

  const client = await createTestClient(context, hostId, { name: 'ReliableTester' });
  await setupSnapshotChannel(client);

  // Welcome already arrived during createTestClient — verify it
  const welcome = await client.lastMessage('Welcome');
  expect(welcome).not.toBeNull();
  expect(welcome.data?.ship_stations).toBeDefined();

  await client.send('SelectStation', { station: 'Helm' });
  await client.send('SetReady', { ready: true });
  await client.page.waitForFunction(
    () => window.__reliableMessages?.some((m) => m.type === 'GameStarted'),
    undefined,
    { timeout: 10_000 },
  );
  const gameStartedRoutes = await client.page.evaluate(() => ({
    reliable: (window.__reliableMessages || []).filter((m) => m.type === 'GameStarted').length,
    snapshot: (window.__snapshotMessages || []).filter((m) => m.type === 'GameStarted').length,
  }));
  expect(gameStartedRoutes.reliable).toBeGreaterThan(0);
  expect(gameStartedRoutes.snapshot).toBe(0);

  await client.close();
});

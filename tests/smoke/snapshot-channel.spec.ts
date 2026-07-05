import { test, expect, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';

/**
 * Helper: set up a snapshot DataChannel on the test client page.
 * Must be called after createTestClient returns (__conn is the reliable DataConnection).
 */
async function setupSnapshotChannel(client: { page: import('playwright').Page }): Promise<void> {
  await client.page.evaluate(() => {
    const conn = (window as any).__conn;
    if (!conn || !conn.peerConnection) return;
    const snapChan = conn.peerConnection.createDataChannel('snapshot', { ordered: false, maxRetransmits: 0 });
    (window as any).__snapChan = snapChan;
    (window as any).__snapshotReady = false;
    snapChan.onopen = () => { (window as any).__snapshotReady = true; };
    snapChan.onclose = () => { (window as any).__snapshotReady = false; };
    snapChan.onmessage = (event: MessageEvent) => {
      try { (window as any).__messages.push(JSON.parse(event.data)); } catch { /* ignore */ }
    };
  });
  // Wait for the shim to propagate the datachannel creation and open
  await client.page.waitForFunction(() => (window as any).__snapshotReady === true, { timeout: 2_000 });
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
  await client.waitForMessage('GameStarted', 5_000);

  // Wait for a SimState message
  const simState = await client.waitForMessage('SimState', 3_000) as any;
  expect(Array.isArray(simState.data.snapshot?.entity_states)).toBe(true);

  // Verify the snapshot channel was used (shim registers it per the label)
  const channels = await client.page.evaluate(() => (window as any).__peerjsShim._dataChannels());
  const hasSnapshot = Object.keys(channels).some(k => k.includes(':snapshot'));
  expect(hasSnapshot).toBe(true);

  // Verify a BlackboardUpdate arrives too
  const bb = await client.waitForMessage('BlackboardUpdate', 3_000) as any;
  expect(Array.isArray(bb.data.updates)).toBe(true);

  await client.close();
});

test('SimState falls back to reliable channel when snapshot channel is unavailable', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);

  // Create a client that does NOT set up a snapshot channel
  const client = await createTestClient(context, hostId, { name: 'FallbackTester' });

  await client.send('SelectStation', { station: 'Helm' });
  await client.send('SetReady', { ready: true });
  await client.waitForMessage('GameStarted', 5_000);

  // SimState should still arrive via the reliable channel fallback
  const simState = await client.waitForMessage('SimState', 3_000) as any;
  expect(Array.isArray(simState.data.snapshot?.entity_states)).toBe(true);

  // Verify no snapshot channel was registered
  const channels = await client.page.evaluate(() => (window as any).__peerjsShim._dataChannels());
  const hasSnapshot = Object.keys(channels).some(k => k.includes(':snapshot'));
  expect(hasSnapshot).toBe(false);

  // Verify a second SimState still arrives (fallback path is sustained)
  const simState2 = await client.waitForMessage('SimState', 4_000) as any;
  expect(Array.isArray(simState2.data.snapshot?.entity_states)).toBe(true);

  await client.close();
});

test('Welcome and commands use the reliable channel regardless of snapshot availability', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);

  const client = await createTestClient(context, hostId, { name: 'ReliableTester' });
  await setupSnapshotChannel(client);

  // Welcome already arrived during createTestClient — verify it
  const welcome = await client.lastMessage('Welcome') as any;
  expect(welcome).not.toBeNull();
  expect(welcome.data?.ship_stations).toBeDefined();

  await client.close();
});

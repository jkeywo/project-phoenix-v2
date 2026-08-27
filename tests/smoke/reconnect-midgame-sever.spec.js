// Issue #614 — auto-reconnect with backoff, exercised via a severed (not
// closed) DataChannel mid-game.
//
// Unlike reconnect.spec.js (which simulates a browser *refresh* — the client
// page itself closes and a brand-new page reconnects with the same token),
// this test simulates a silently dropped DataChannel — e.g. a phone's radio
// sleeping — while the SAME client.html page stays alive. That is exactly the
// case the Phoenix transport's reconnect-with-backoff loop exists for:
// the page never reloads, so it's the manager's own retry loop (not a fresh
// page load) that must re-establish the connection, re-send Identify, and
// land back on the same station.
//
// Drives the REAL client.html DOM (not the raw createTestClient JS shim) so
// the assertions exercise the actual `gui/rendezvous-transport.js` +
// `setConnectionStatus` UI wiring described in the issue, and the actual
// `gui/sim-state.js` state that consoles render from.
//
// Issue #1112 replaced the mechanism and kept every assertion: the sever is
// now the transport shim's page-scoped kill switch rather than a PeerJS peer
// pair, and the instrumented link is `window.phoenixLink` rather than
// `window.connectionManager`. What is being proved is the #1112 reconnect AC
// almost word for word — same code, same session token, station and projection
// restored, and no five letters typed a second time.

import { test, expect, readHostPeerId, createServerPage } from './fixtures';

// Pull blackboard-backed console state through the same builders used by the
// iframes. Repair hull uses the current `system_hull` shape when available;
// the solo-captain smoke scenario also accepts the active captain console's
// blackboard-backed state as the "current system state" signal.
function consoleSystemState(page) {
  return page.evaluate(() => {
    const state = window.simState;
    const buildConsoleState = window.buildConsoleState;
    if (!state || typeof buildConsoleState !== 'function') {
      return { repairHull: [], captainBlackboard: null, captainState: null };
    }
    try {
      const repair = JSON.parse(buildConsoleState('repair', state));
      const captain = JSON.parse(buildConsoleState('captain', state));
      return {
        repairHull: repair.system_hull ?? [],
        captainBlackboard: state.blackboards?.captain ?? null,
        captainState: captain,
      };
    } catch (_) {
      return { repairHull: [], captainBlackboard: null, captainState: null };
    }
  });
}

function hasConsoleSystemState() {
  const state = window.simState;
  const buildConsoleState = window.buildConsoleState;
  if (!state || typeof buildConsoleState !== 'function') return false;
  try {
    const repair = JSON.parse(buildConsoleState('repair', state));
    return (repair.system_hull ?? []).length > 0 || !!state.blackboards?.captain;
  } catch (_) {
    return false;
  }
}

test('sever + revive mid-game: seat restored and console reflects current system state', async ({ context }) => {
  test.setTimeout(60_000);

  const serverPage = await createServerPage(context);
  const hostId = await readHostPeerId(serverPage);

  // Solo crew: Captain covers every console at 1P, so a single player can
  // Engage and the game moves into InProgress.
  const client = await context.newPage();

  // Instrument the WIRE before anything connects, so we can assert Identify
  // was re-sent after revive.
  //
  // Identify is the transport's own frame, not something the page sends
  // through `window.phoenixLink` — wrapping the link would record commands and
  // miss the one message under test. This wraps the peer factory the transport
  // takes its RTCPeerConnections from, which records everything this phone puts
  // on a DataChannel and survives a reconnect: the joiner builds a NEW peer
  // connection and NEW channels for every attempt, so anything wrapped lower
  // down would be thrown away by exactly the event being tested.
  //
  // It has to be installed BEFORE the page connects, because the transport
  // resolves its factories once at construction; the baseline captured below is
  // what separates the reconnect's Identify from the original one.
  await client.addInitScript(() => {
    window.__sentMessages = [];
    const install = () => {
      const factories = window.PhoenixTransportFactories;
      if (!factories) return false;
      const makePeer = factories.peer;
      factories.peer = (config) => {
        const pc = makePeer(config);
        const create = pc.createDataChannel.bind(pc);
        pc.createDataChannel = (label, init) => {
          const channel = create(label, init);
          const send = channel.send.bind(channel);
          channel.send = (payload) => {
            try {
              const msg = JSON.parse(payload);
              window.__sentMessages.push({ type: msg.type, data: msg.data, channel: label });
            } catch (_) { /* not JSON — not ours */ }
            return send(payload);
          };
          return channel;
        };
        return pc;
      };
      return true;
    };
    if (!install()) document.addEventListener('DOMContentLoaded', install, { once: true });
  });

  await client.goto(`/client/#${hostId}`);
  await client.waitForSelector('#station-list .station-row', { timeout: 15_000 });
  await client.click('#station-list .station-row:has-text("Captain") button.claim-btn');
  await client.waitForSelector('#ready-btn:not([style*="display: none"])', { timeout: 5_000 });
  await client.click('#ready-btn');

  // Ready hands the station over and the client transitions from the lobby
  // panel into the console view — this is the same "game started" signal
  // midgame-claim-buttons.spec.js asserts on. Solo Captain lands on the
  // captain console.
  await expect(client.locator('#captain-ui')).toHaveClass(/active/, { timeout: 10_000 });

  // Let a couple of 10Hz BlackboardUpdate ticks land so window.simState has
  // real pre-sever state to compare against.
  await client.waitForFunction(
    hasConsoleSystemState,
    undefined,
    { timeout: 10_000 },
  );

  // Baseline the Identify log, and wrap simState.apply() — the first stop for
  // every inbound ServerMessage — so only the state messages that arrive after
  // the RECONNECT's Identify are recorded. Without the baseline the phone's
  // original Identify would arm the recorder and the final assertion could pass
  // on state cached before the sever.
  const { myToken } = await client.evaluate(() => {
    const identifies = () => window.__sentMessages.filter((m) => m.type === 'Identify').length;
    const baseline = identifies();
    window.__identifyBaseline = baseline;
    window.__postReconnectStateMessages = [];
    const simState = window.simState;
    const originalApply = simState.apply.bind(simState);
    simState.apply = (msg) => {
      if (
        identifies() > baseline &&
        (msg?.type === 'BlackboardUpdate' || msg?.type === 'SystemHullUpdate' || msg?.type === 'SimState')
      ) {
        window.__postReconnectStateMessages.push({
          type: msg.type,
          updateCount: Array.isArray(msg.data?.updates) ? msg.data.updates.length : undefined,
          entryCount: Array.isArray(msg.data?.entries) ? msg.data.entries.length : undefined,
        });
      }
      return originalApply(msg);
    };
    return { myToken: sessionStorage.getItem('session-token') };
  });

  expect(myToken).toBeTruthy();

  const stateBeforeSever = await consoleSystemState(client);
  expect(stateBeforeSever.repairHull.length > 0 || !!stateBeforeSever.captainBlackboard).toBe(true);

  // ── Sever ──────────────────────────────────────────────────────────────
  // Kill this page's DataChannels without either side calling close(). The
  // shim posts the close to the far end too, so the server-side
  // conn.on('close') path runs. It also holds this page off the network until
  // the explicit revive below, so the transport's own backoff loop cannot race
  // past the visible-retry assertion by reconnecting on its own.
  await client.evaluate(() => window.__transportShim.sever());

  // The UI must show the disconnected/retrying state with a visible
  // "Retry now" control — this is the acceptance-criteria affordance, not
  // just an internal state flag.
  await client.waitForFunction(
    () => document.getElementById('retry-now-btn')?.classList.contains('visible') === true,
    undefined,
    { timeout: 5_000 },
  );
  await expect(client.locator('#conn-label')).toContainText('reconnecting', { timeout: 5_000 });

  // ── Revive ─────────────────────────────────────────────────────────────
  // Bring the shim link back, then trigger the "retry now" control directly
  // (real DOM click) rather than waiting out the full backoff schedule. This
  // exercises the same retryNow() path a real user's tap would, and keeps the
  // test fast/deterministic.
  await client.evaluate(() => window.__transportShim.revive());
  await client.click('#retry-now-btn');

  // Reconnect re-establishes the DataChannel, which must re-send Identify —
  // with the SAME session token, and without the guest re-entering the join
  // code (#1112 AC3).
  await client.waitForFunction(
    () => window.__sentMessages.filter((m) => m.type === 'Identify').length
      > window.__identifyBaseline,
    undefined,
    { timeout: 15_000 },
  );
  const identifyCalls = await client.evaluate(
    () => window.__sentMessages.filter((m) => m.type === 'Identify'),
  );
  expect(identifyCalls.length).toBeGreaterThanOrEqual(2);
  expect(identifyCalls[identifyCalls.length - 1].data.token).toBe(myToken);
  // …and it went out on the reliable channel, which is where the host's
  // Identify gate reads it.
  expect(identifyCalls[identifyCalls.length - 1].channel).toBe('reliable');

  // Prove a fresh post-reconnect system-state message landed through the
  // client state pipeline after Identify. Without this, the final simState
  // assertion could pass using state cached before the sever.
  await client.waitForFunction(
    () => window.__postReconnectStateMessages?.some(
      (m) => m.type === 'BlackboardUpdate' || m.type === 'SystemHullUpdate',
    ),
    undefined,
    { timeout: 5_000 },
  );
  const postReconnectStateMessages = await client.evaluate(
    () => window.__postReconnectStateMessages,
  );
  expect(postReconnectStateMessages.some(
    (m) => m.type === 'BlackboardUpdate' || m.type === 'SystemHullUpdate',
  )).toBe(true);

  // Connection status returns to normal (dot green, retry button hidden).
  await client.waitForFunction(
    () => document.getElementById('retry-now-btn')?.classList.contains('visible') === false,
    undefined,
    { timeout: 10_000 },
  );

  // ── Seat restored ──────────────────────────────────────────────────────
  // The reconnecting Identify carries the same token, so the server restores
  // the same station via the existing seat/rating-restore flow (unchanged by
  // this issue — see lobby handler + Welcome). Confirm via window.lobbyState,
  // which client.html's handleMessage mirrors on every Welcome.
  await client.waitForFunction(
    (token) => {
      const players = window.lobbyState?.players ?? [];
      const me = players.find((p) => p.token === token);
      return !!me && me.station === 'captain';
    },
    myToken,
    { timeout: 10_000 },
  );

  // ── Console reflects current system state within one broadcast tick ────
  // The registered reconnect runner pushes a fresh BlackboardUpdate (among
  // others) targeted at the reconnecting token immediately after Welcome.
  // Confirm gui/sim-state.js actually applied it — the repair console's
  // per-system hull list must be repopulated (not stuck empty/stale from
  // the moment of sever).
  await client.waitForFunction(
    hasConsoleSystemState,
    undefined,
    { timeout: 5_000 },
  );
  const stateAfterRevive = await consoleSystemState(client);
  expect(stateAfterRevive.repairHull.length > 0 || !!stateAfterRevive.captainBlackboard).toBe(true);
  if (stateBeforeSever.repairHull.length > 0) {
    expect(stateAfterRevive.repairHull.length).toBe(stateBeforeSever.repairHull.length);
  } else {
    expect(stateAfterRevive.captainState?.viewscreen_system_id)
      .toBe(stateBeforeSever.captainState?.viewscreen_system_id);
  }

  await client.close();
});

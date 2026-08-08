// Issue #614 — auto-reconnect with backoff, exercised via a severed (not
// closed) DataChannel mid-game.
//
// Unlike reconnect.spec.js (which simulates a browser *refresh* — the client
// page itself closes and a brand-new page reconnects with the same token),
// this test simulates a silently dropped DataChannel — e.g. a phone's radio
// sleeping — while the SAME client.html page stays alive. That is exactly the
// case connection-manager.js's new reconnect-with-backoff logic exists for:
// the page never reloads, so it's the manager's own retry loop (not a fresh
// page load) that must re-establish the connection, re-send Identify, and
// land back on the same station.
//
// Drives the REAL client.html DOM (not the raw createTestClient JS shim) so
// the assertions exercise the actual `gui/connection-manager.js` +
// `setConnectionStatus` UI wiring described in the issue, and the actual
// `gui/sim-state.js` state that consoles render from.

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

  // Instrument: record every ConnectionManager.send() call so we can assert
  // Identify was re-sent after revive. Also wrap simState.apply(), which is
  // the first stop for every inbound ServerMessage, and record only fresh
  // system-state messages that arrive after that reconnect Identify.
  const { myPeerId, myToken } = await client.evaluate(() => {
    window.__sentMessages = [];
    window.__recordPostReconnectState = false;
    window.__postReconnectStateMessages = [];
    const cm = window.connectionManager;
    const originalSend = cm.send.bind(cm);
    cm.send = (type, data) => {
      window.__sentMessages.push({ type, data });
      if (type === 'Identify') {
        window.__recordPostReconnectState = true;
      }
      return originalSend(type, data);
    };
    const simState = window.simState;
    const originalApply = simState.apply.bind(simState);
    simState.apply = (msg) => {
      if (
        window.__recordPostReconnectState &&
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
    return {
      myPeerId: cm.peer?.id,
      myToken: sessionStorage.getItem('session-token'),
    };
  });

  expect(myPeerId).toBeTruthy();
  expect(myToken).toBeTruthy();

  const stateBeforeSever = await consoleSystemState(client);
  expect(stateBeforeSever.repairHull.length > 0 || !!stateBeforeSever.captainBlackboard).toBe(true);

  // ── Sever ──────────────────────────────────────────────────────────────
  // Kill the DataChannel on both ends via the shim's test-only API. The shim
  // broadcasts this control message to every page context, so the server-side
  // conn.on('close') path runs too. It also holds the link offline until the
  // explicit revive below, preventing auto-retry from racing past the visible
  // retry assertion.
  await client.evaluate(
    ({ myPeerId, hostId }) => {
      window.__peerjsShim.severConnection(myPeerId, hostId);
    },
    { myPeerId, hostId },
  );

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
  await client.evaluate(
    ({ myPeerId, hostId }) => {
      window.__peerjsShim.reviveConnection(myPeerId, hostId);
    },
    { myPeerId, hostId },
  );
  await client.click('#retry-now-btn');

  // Reconnect re-establishes the DataChannel, which must re-send Identify
  // (bug #1 from the issue: _identSent must reset on close).
  await client.waitForFunction(
    () => window.__sentMessages?.some((m) => m.type === 'Identify'),
    undefined,
    { timeout: 10_000 },
  );
  const identifyCalls = await client.evaluate(
    () => window.__sentMessages.filter((m) => m.type === 'Identify'),
  );
  expect(identifyCalls.length).toBeGreaterThanOrEqual(1);
  expect(identifyCalls[identifyCalls.length - 1].data.token).toBe(myToken);

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
  // Issue #613's resync_for_token pushes a fresh BlackboardUpdate (among
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

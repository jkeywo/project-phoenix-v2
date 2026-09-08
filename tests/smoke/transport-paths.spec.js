// Issue #1113 — forcing each transport path, in a real browser.
//
// Phoenix has three ways onto the wire and tries them in order, so on a healthy
// network the first one always wins and the other two are never exercised.
// gui/transport-levers.js turns that chain into something a test can pin, and
// these specs pin it: a direct join, a TURN-only join, a signalling reconnect,
// and a join carried entirely by the rendezvous service's WebSocket relay.
//
// ── What these specs honestly prove, and what they do not ───────────────────
//
// The transport under test is real: the shipped client page, the shipped host
// page, the shipped gui/rendezvous-transport.js, and the REAL
// worker-rendezvous registry running inside the host page. What is fake is the
// wire — tests/smoke/rendezvous-shim.js pairs two pages' DataChannels over a
// BroadcastChannel and terminates the rendezvous socket in-process, because CI
// has neither WebRTC nor a deployed worker.
//
// So, per path:
//
//   direct     genuinely exercised end to end: two channels negotiate and the
//              ordinary Identify→Welcome flow runs over them.
//   TURN-only  SHIM-LEVEL. The fake peer connection has no ICE to restrict, so
//              it links whatever `iceTransportPolicy` says. What is proved is
//              that the lever reaches BOTH peer connections — which is the part
//              that can regress silently, and the part the field session then
//              relies on. That an actual TURN allocation succeeds is
//              docs/acceptance/1113-networks.md scenario 3, against the
//              deployed service, and nothing in CI can stand in for it.
//   reconnect  genuinely exercised: the link really closes, the joiner really
//              re-resolves the same code, and the diagnostics it emits on the
//              way are what this spec reads.
//   ws-relay   genuinely exercised end to end, over the real registry's real
//              relay hub — but PINNED. The automatic escalation (four direct
//              attempts on the 8/16/30 s ladder, then the fallback) is proved
//              in tests/client/rendezvous-transport.test.js under fake timers,
//              because at real speed it is ninety seconds of waiting.

import { test, expect, waitForWasmReady, waitForJoinCode } from './fixtures';
import { ts } from './strings';

async function bootHost(context, search = '?scenario=assets/worlds/default.toml') {
  const page = await context.newPage();
  await page.goto(`/${search}`);
  await waitForWasmReady(page);
  await waitForJoinCode(page, 'join-code', 30_000);
  return page;
}

const joinCodeOn = (page) => page.evaluate(() => document.getElementById('join-code').textContent);

/** Open the client page with a transport lever set, and type the code in. */
async function joinWith(context, code, search = '') {
  const page = await context.newPage();
  await page.goto(`/client/${search}`);
  await expect(page.locator('#join-entry')).toBeVisible();
  await page.fill('#join-code-input', code);
  await page.click('#join-submit-btn');
  return page;
}

const waitForConnected = (page) =>
  page.waitForFunction(
    (expected) => document.getElementById('status')?.textContent === expected,
    ts('client.status_connected'),
    { timeout: 30_000 },
  );

const diagText = (page) =>
  page.evaluate(() => document.getElementById('conn-diag')?.textContent ?? '');

const channelsOn = (page) => page.evaluate(() => window.__transportShim.dataChannels());
const peerConfigsOn = (page) => page.evaluate(() => window.__transportShim.peerConfigs());

/**
 * The part of the host's "carried by the join service" line that does not
 * depend on the peer id, taken FROM strings.csv rather than retyped.
 *
 * Two assertions in this file used to hand-copy a fragment of
 * `server.client_ws_relay`, which meant a wording edit broke them with no
 * signal from check-strings.mjs and no way to notice the drift.
 */
const WS_RELAY_LINE = ts('server.client_ws_relay', { id: 'PEERID' }).split('PEERID').pop();

test('a direct join negotiates both channels and never reaches for a fallback', { tag: '@core' }, async ({ context }) => {
  // PINNED to direct on both ends, which is what makes this the direct-path
  // spec rather than "whatever the shim happened to build". `?transport=direct`
  // withholds the STUN/TURN list from the peer connection — the only spelling
  // of "no relay in this path", since iceTransportPolicy has no such value —
  // and switches off the WebSocket fallback.
  const host = await bootHost(context, '?scenario=assets/worlds/default.toml&transport=direct');
  const client = await joinWith(context, await joinCodeOn(host), '?transport=direct');
  await waitForConnected(client);

  // The pin reached BOTH peer connections. Until #1113's review this mode gave
  // `iceTransportPolicy: 'all'` and a full server list — identical to auto — so
  // the one lever whose job is proving a direct link proved nothing.
  for (const configs of [await peerConfigsOn(client), await peerConfigsOn(host)]) {
    expect(configs.length).toBeGreaterThan(0);
    expect(configs.every((c) => c.iceServers === 0)).toBe(true);
  }

  // Two channels, from ONE negotiation, with the lossy one really unordered
  // and non-retransmitting rather than the reliable one under another name.
  const channels = Object.values(await channelsOn(client));
  const reliable = channels.find((c) => c.side === 'offer' && c.maxRetransmits === null);
  const snapshot = channels.find((c) => c.side === 'offer' && c.maxRetransmits === 0);
  expect(reliable).toMatchObject({ readyState: 'open', ordered: true });
  expect(snapshot).toMatchObject({ readyState: 'open', ordered: false });

  // Nothing degraded, so neither readout says anything about the WebSocket
  // relay. Asserting the ABSENCE is the point: these lines are how a degraded
  // path announces itself, and a spec that only ever checks for them when they
  // are expected cannot notice them appearing by accident.
  expect(await diagText(client)).not.toContain(ts('client.diag_ws_relay'));
  expect(await diagText(host)).not.toContain(WS_RELAY_LINE);
  // …and both surfaces SAY the transport was pinned, so a field failure is not
  // blamed on the network when the link was restricted by hand.
  expect(await diagText(client)).toContain(ts('client.diag_transport_pinned', { mode: 'direct' }));
  expect(await diagText(host)).toContain(ts('server.transport_pinned', { mode: 'direct' }));
});

test('?forceRelay pins both ends of the negotiation to relay candidates', async ({ context }) => {
  // SHIM-LEVEL, deliberately — see this file's header. The fake peer connection
  // has no ICE, so it cannot refuse a host candidate; what must not regress is
  // that the lever reaches the RTCPeerConnection config on BOTH pages, because
  // ICE only negotiates a relayed pair when both ends offer relay candidates,
  // and a lever that reached only the phone would quietly prove nothing in the
  // field session that relies on it.
  const host = await bootHost(context, '?scenario=assets/worlds/default.toml&forceRelay=1');
  const client = await joinWith(context, await joinCodeOn(host), '?forceRelay=1');
  await waitForConnected(client);

  const clientConfigs = await peerConfigsOn(client);
  expect(clientConfigs.length).toBeGreaterThan(0);
  expect(clientConfigs.every((c) => c.iceTransportPolicy === 'relay')).toBe(true);

  const hostConfigs = await peerConfigsOn(host);
  expect(hostConfigs.length).toBeGreaterThan(0);
  expect(hostConfigs.every((c) => c.iceTransportPolicy === 'relay')).toBe(true);

  // The mirror image of the direct pin, and the reason that pin's assertion is
  // not vacuous: TURN-only KEEPS the server list (there is nothing to allocate
  // from without it) where direct withholds it.
  expect(clientConfigs.every((c) => c.iceServers > 0)).toBe(true);
  expect(hostConfigs.every((c) => c.iceServers > 0)).toBe(true);

  // And both readouts SAY the transport was restricted, so a failure in the
  // field is not blamed on the network when the link was pinned by hand.
  await expect
    .poll(() => diagText(client), { timeout: 15_000 })
    .toContain(ts('client.diag_transport_pinned', { mode: 'turn' }));
  expect(await diagText(host)).toContain(ts('server.transport_pinned', { mode: 'turn' }));
});

test('a dropped link reports its reconnect attempts on the diagnostics readout', async ({ context }) => {
  const host = await bootHost(context);
  const client = await joinWith(context, await joinCodeOn(host));
  await waitForConnected(client);

  // The phone's radio sleeping: channels close on both ends without either
  // side calling close(), and this page cannot reach the service either, so
  // its automatic retries keep failing and cannot race the assertion.
  await client.evaluate(() => window.__transportShim.sever());

  // The readout is what a guest can act on while the status line only says
  // "reconnecting": which attempt, and how far it got.
  await expect
    .poll(() => diagText(client), { timeout: 30_000 })
    .toMatch(/attempt \d/);

  await client.evaluate(() => window.__transportShim.revive());
  // Restoring the radio lets the pending automatic retry complete. Clicking
  // Retry now here races that success, which correctly hides the button.
  await waitForConnected(client);
});

test('the WebSocket relay carries a whole session when WebRTC is off the table', async ({ context }) => {
  const host = await bootHost(context);
  const client = await joinWith(context, await joinCodeOn(host), '?transport=ws-relay');
  await waitForConnected(client);

  // No peer connection was ever built: the game did not go over WebRTC at all.
  expect(await channelsOn(client)).toEqual({});
  expect(await peerConfigsOn(client)).toEqual([]);

  // The host admitted this phone as an ORDINARY player — same Identify gate,
  // same current connection owner — which is what "the fallback does not fork the game
  // protocol" has to mean to be worth anything.
  const token = await client.evaluate(() => sessionStorage.getItem('session-token'));
  expect(token).toBeTruthy();
  await host.waitForFunction(
    (t) => {
      try {
        // eslint-disable-next-line no-eval
        return (0, eval)('hostConnections').targets(`token:${t}`, 'reliable').length === 1;
      } catch { return false; }
    },
    token,
    { timeout: 15_000 },
  );

  // Both surfaces say the link is degraded, with the two different things the
  // two readers need: the guest is told to expect lag, the operator is told
  // which crew member has no direct link.
  expect(await diagText(client)).toContain(ts('client.diag_ws_relay'));
  await expect
    .poll(() => diagText(host), { timeout: 15_000 })
    .toContain(WS_RELAY_LINE);
});

test('a snapshot channel that opened before Identify is promoted when the token lands', async ({ context }) => {
  // The lossy channel can finish negotiating on EITHER side of the Identify
  // that names this connection's token, and the host's per-token routing selection
  // has to end up holding it whichever way round it happened. In the ordinary
  // WebRTC join it is always the early one: both channels come off one
  // negotiation and the compatibility handshake has to complete before the
  // phone may send Identify at all. So this is the ordering that runs on every
  // real join, and nothing covered it after the peerjs shim was retired — the
  // behaviour lives in gui/host-peer-routing.js's current-owner selection,
  // where the snapshot channel belongs to its physical incarnation and an
  // old connection cannot replace the current owner's route.
  const host = await bootHost(context);
  const client = await joinWith(context, await joinCodeOn(host));
  await waitForConnected(client);

  const token = await client.evaluate(() => sessionStorage.getItem('session-token'));
  expect(token).toBeTruthy();

  // The lossy channel opened first…
  const channels = Object.values(await channelsOn(client));
  expect(channels.find((c) => c.side === 'offer' && c.maxRetransmits === 0))
    .toMatchObject({ readyState: 'open' });

  // …and the token, once it arrived, adopted it. Without the promotion the
  // snapshot route is absent and every snapshot for this player silently falls
  // back to the reliable channel for the rest of the mission — a regression
  // with no error, no log line and no visible symptom short of head-of-line
  // stalls on a bad radio.
  const bound = await host.waitForFunction(
    (t) => {
      try {
        // eslint-disable-next-line no-eval
        const routes = (0, eval)('hostConnections').targets(`token:${t}`, 'snapshot');
        const chan = routes.length === 1 ? routes[0] : null;
        return chan ? { label: chan.label, readyState: chan.readyState } : false;
      } catch { return false; }
    },
    token,
    { timeout: 15_000 },
  );
  // The LOSSY one, not the reliable channel under another name.
  expect(await bound.jsonValue()).toMatchObject({ label: 'snapshot', readyState: 'open' });
});

test('the diagnostics dump is offered on both pages for the field sessions', async ({ context }) => {
  // docs/acceptance/1113-networks.md asks a tester to paste this into the issue
  // thread, so the control has to be there and populated on both pages.
  const host = await bootHost(context);
  const client = await joinWith(context, await joinCodeOn(host));
  await waitForConnected(client);

  for (const page of [host, client]) {
    const button = page.locator('#conn-diag-copy');
    await expect(button).toBeVisible();
    await expect(button).not.toBeEmpty();
  }
});

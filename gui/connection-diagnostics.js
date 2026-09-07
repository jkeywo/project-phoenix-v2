/**
 * gui/connection-diagnostics.js — what the `#conn-diag` readout says, and the
 * dump a field tester pastes into an issue (issue #1113).
 *
 * ## Why this is a module and not two more closures in two HTML files
 *
 * Both pages already had a `renderConnDiag`, written inline, and
 * gui/page-chrome.js deliberately unified only the mechanical half of them
 * (guard for a missing element, join the lines, set `textContent`) because the
 * two line SETS are genuinely different: the host summarises every connected
 * phone, the client summarises this one device. That reasoning still holds and
 * this module does not undo it — there are still two line builders.
 *
 * What changed is that #1113 made the line sets say considerably more (which
 * rung of the transport ladder is in use, which TURN provider actually carried
 * the traffic, which stage a timeout happened at, whether the relay is
 * shedding), and asked for a copy-pasteable dump of the same state. Three
 * things follow: the logic is worth testing, an inline closure in a 4,000-line
 * HTML file is not testable, and the dump and the readout must not be able to
 * disagree about what is happening. So the STATE and both readings of it live
 * here, and each page keeps only the wiring that feeds it.
 *
 * ## What an actionable line looks like
 *
 * Every line here answers "and what do I do about it" for somebody standing in
 * a room with a phone that will not connect:
 *
 *   no relay at all           → mobile networks will fail; fix the credential
 *                               worker (docs/delivery-checklist.md §3)
 *   the free fallback relay   → the credential worker is unreachable; the
 *                               session works but is shared and rate-limited
 *   carried by the service    → this network blocks direct links entirely;
 *                               expect latency, and try another network
 *   shedding snapshots        → the link cannot keep up; commands still land
 *   pinned by a lever         → the transport is deliberately restricted, so a
 *                               failure here may not be the network's fault
 *
 * The READOUT LINES are never prose: each one is a strings.csv id resolved
 * through the `t` the caller passes in (AGENTS.md rule 11). `diagnosticsDump`
 * at the bottom is the deliberate exception and the header used to claim
 * otherwise: its labels are operator prose in a block pasted into an issue
 * thread by one tester and read by another, the same standing that
 * `StampMismatch::detail()` has on the Rust side. It is never shown to a
 * player, and `check-strings.mjs --strict` is green on it for that reason.
 */

/** The transport rung names, as the joiner reports them on its diag events. */
export const TRANSPORT_DIRECT = 'direct';
export const TRANSPORT_WS_RELAY = 'ws-relay';

/**
 * A blank client-side diagnostics state.
 *
 * `relayAvailable`/`relaySource` come from `fetchIceServers()` at boot and
 * never change; everything else is written by `applyClientDiagEvent` as the
 * transport reports on itself.
 */
export function createClientDiagnostics(initial = {}) {
  return {
    attempt: 0,
    ice: '',
    types: [],
    probe: null,
    relayAvailable: false,
    relaySource: null,
    transport: TRANSPORT_DIRECT,
    transportReason: null,
    lever: { mode: 'auto', pinned: false },
    /** The connect-timeout rung this attempt gave up at, if it did. */
    timeoutStage: null,
    signaling: null,
    /** `{ local, remote, protocol, url }` once ICE has chosen a pair. */
    selectedPair: null,
    /**
     * Snapshot frames the relay has shed, per REPORTER.
     *
     * The two ends measure different queues, and BOTH are this device's own
     * uplink, just at different hops — `client` is what this device shed
     * against its own send buffer before a frame ever left it, `service` is
     * what the SERVICE shed of this device's outbound frames at the
     * (host,phone) mailbox before they reached the host. `relay-degraded` is
     * always answered to the SENDER (the one that can slow down), never the
     * recipient, so a phone only ever sees the `service` reporter fire for its
     * OWN traffic — never for the host's snapshots coming down. That is why
     * they are held apart and added rather than folded with `Math.max`: two
     * measurements of the same direction at two hops, not a downlink and an
     * uplink that would double-count one loss. Both numbers are now
     * cumulative; the fold used to take the larger on the stated assumption
     * that both were, which was true of the client's and false of the
     * service's (a per-enqueue delta, 1 essentially always), so the readout
     * sat at "Dropping display updates (1)" however many hundreds were being
     * lost.
     */
    relayDroppedBy: { client: 0, service: 0 },
    /** The rendered total: the sum of the two above. */
    relayDropped: 0,
    ...initial,
  };
}

/**
 * Fold one `onDiag` event from gui/rendezvous-transport.js into the state.
 *
 * Mutates and returns `state`, because the pages hold one long-lived object and
 * repaint from it; a copying fold would only add a reassignment at every call
 * site for no gain.
 */
export function applyClientDiagEvent(state, event) {
  const e = event || {};
  switch (e.event) {
    case 'attempt':
      // A new attempt invalidates everything the last one observed. Leaving a
      // stale ICE state or candidate list on screen would have the readout
      // describing an attempt that is already over.
      state.attempt = e.attempt;
      state.ice = '';
      state.types = [];
      state.timeoutStage = null;
      state.selectedPair = null;
      break;
    case 'transport':
      state.transport = e.transport;
      if (e.reason) state.transportReason = e.reason;
      if (e.mode !== undefined || e.pinned !== undefined) {
        state.lever = { mode: e.mode || state.lever.mode, pinned: !!e.pinned };
      }
      break;
    case 'ice-state':
      state.ice = e.state;
      break;
    case 'candidates':
      state.types = e.types || [];
      break;
    case 'signaling':
      state.signaling = e.state;
      break;
    case 'timeout':
      state.ice = 'timeout';
      state.timeoutStage = e.attempt;
      break;
    case 'open':
      state.ice = 'connected';
      state.timeoutStage = null;
      break;
    case 'selected-pair':
      state.selectedPair = e.pair || null;
      break;
    case 'relay-degraded': {
      // Two reporters, two different queues, both cumulative and both this
      // device's own uplink — so keep them apart and add. `from` is 'client'
      // (this device's own send buffer) or 'service' (the service shedding
      // this device's own outbound frames at the (host,phone) mailbox —
      // `relay-degraded` always answers the SENDER, so this is never the
      // host's snapshots on their way down); an event with neither is this
      // device's, which is where the counter started.
      const source = e.from === 'service' ? 'service' : 'client';
      if (!state.relayDroppedBy) state.relayDroppedBy = { client: 0, service: 0 };
      state.relayDroppedBy[source] = Math.max(
        state.relayDroppedBy[source] || 0,
        e.dropped || 0,
      );
      state.relayDropped = (state.relayDroppedBy.client || 0)
        + (state.relayDroppedBy.service || 0);
      break;
    }
    default:
      break;
  }
  return state;
}

/**
 * The relay-configuration line, shared by both pages because the question is
 * the same one on each: is there a TURN relay, whose is it, and does it work
 * from here? Returns a string id and params, or null when there is nothing
 * worth saying.
 */
function relayLine(state, ids) {
  if (!state.relayAvailable) return ids.noRelay;
  if (state.probe === 'unreachable') return ids.relayFail;
  if (state.relaySource === 'openrelay') {
    return state.probe === 'reachable' ? ids.fallbackOk : ids.fallback;
  }
  return state.probe === 'reachable' ? ids.relayOk : null;
}

/**
 * The lines under a phone's join screen.
 *
 * Order is deliberate and is the order a person diagnoses in: what relay do I
 * have, which path am I actually on, how is this attempt going, and what is the
 * link doing to my traffic.
 */
export function clientDiagnosticsLines(state, t) {
  const lines = [];
  const relay = relayLine(state, {
    noRelay: 'client.diag_no_relay',
    relayFail: 'client.diag_relay_fail',
    fallbackOk: 'client.diag_relay_fallback_ok',
    fallback: 'client.diag_relay_fallback',
    relayOk: 'client.diag_relay_ok',
  });
  if (relay) lines.push(t(relay));

  // Only ever said when it is NEWS. On a healthy join the first rung wins, and
  // a line reading "path: direct WebRTC" under every successful connection is
  // noise a reader learns to skip past — which is how the lines that DO matter
  // get skipped past too.
  if (state.transport === TRANSPORT_WS_RELAY) lines.push(t('client.diag_ws_relay'));
  if (state.lever && state.lever.pinned) {
    lines.push(t('client.diag_transport_pinned', { mode: state.lever.mode }));
  }

  if (state.attempt > 0) {
    lines.push(t('client.diag_stage', {
      attempt: state.attempt,
      ice: state.ice || '—',
      types: state.types.length ? state.types.join(', ') : t('client.diag_none'),
    }));
  }
  if (state.selectedPair) {
    lines.push(t('client.diag_selected', selectedPairParams(state.selectedPair, t)));
  }
  if (state.relayDropped > 0) {
    lines.push(t('client.diag_relay_shedding', { n: state.relayDropped }));
  }
  return lines;
}

/**
 * The lines under the viewscreen's QR.
 *
 * `state.peers` is `[[shortId, iceState], …]` — the host's own map of joiners
 * that are NOT healthy, plus any being carried by the service, which is a
 * working-but-degraded state the operator should still see.
 *
 * `state.shedding` is `[[shortId, count], …]`, held SEPARATELY and rendered as
 * its own line. A relayed peer that starts shedding is two facts at once, and
 * folding the count into the peer-state map made it one: the state string
 * became a fabricated `relay-shedding-3`, which is not one of ICE's five, so
 * the row stopped saying "carried by the join service" — the exact line the
 * acceptance kit tells a tester to look for — and printed a raw technical
 * token in an operator-facing sentence instead.
 */
export function hostDiagnosticsLines(state, t) {
  const lines = [];
  const relay = relayLine(state, {
    noRelay: 'server.no_relay_warning',
    relayFail: 'server.no_relay_warning',
    fallbackOk: 'server.relay_fallback_notice',
    fallback: 'server.relay_fallback_notice',
    relayOk: null,
  });
  if (relay) lines.push(t(relay));

  if (state.lever && state.lever.pinned) {
    lines.push(t('server.transport_pinned', { mode: state.lever.mode }));
  }

  // Why there is no typed code on screen. The operator's next move
  // differs by case. Three sentences, not two, since issue #1115: a lost
  // service that is retrying either has a code worth reclaiming (this host has
  // held one before — surfaced here as state.resuming, off rendezvousHost.
  // resuming) or does not (its very first registration never got that far).
  // Both are honest about what happens next; the old wording promised a NEW
  // code unconditionally, which stopped being true the moment reclaim started
  // working. Anything not retrying means nobody joins until the service comes
  // back.
  if (state.fault) {
    const reclaiming = state.retrying && state.reregister && !!state.resuming;
    lines.push(t(
      !state.retrying || !state.reregister ? 'server.join.rendezvous_lost'
        : reclaiming ? 'server.join.rendezvous_reclaiming'
          : 'server.join.rendezvous_reregistering',
    ));
  }

  for (const [id, peerState] of state.peers || []) {
    lines.push(peerState === TRANSPORT_WS_RELAY
      ? t('server.client_ws_relay', { id })
      : t('server.client_ice_state', { id, state: peerState }));
  }
  for (const [id, n] of state.shedding || []) {
    if (!n) continue;
    lines.push(t('server.client_relay_shedding', { id, n }));
  }
  return lines;
}

/** The readable half of a selected candidate pair. */
function selectedPairParams(pair, t) {
  const none = t('client.diag_none');
  return {
    local: pair.local || none,
    remote: pair.remote || none,
    relay: pair.url || pair.protocol || none,
  };
}

/**
 * The dump a field tester copies into the issue thread.
 *
 * Deliberately plain text with fixed labels rather than JSON: it is pasted into
 * a GitHub comment by somebody standing in a car park, read by somebody else,
 * and it has to survive both. It carries no join code, no session token and no
 * peer id beyond the short prefixes already on screen — a diagnostics dump that
 * leaked the code would be a dump nobody could safely paste.
 *
 * What is deliberately NOT in it: the network's name. Neither page has a field
 * for one and nothing could invent it, so the row was always omitted and the
 * acceptance kit's worked example showed output the product cannot produce.
 * docs/acceptance/1113-networks.md asks the tester to write the network above
 * the pasted block instead, which is where a human sentence belongs anyway.
 *
 * @param {object} state a client or host diagnostics state
 * @param {object} meta `{ page, service, build, now }` — what the page knows
 *   about itself
 */
export function diagnosticsDump(state, meta = {}) {
  const rows = [];
  const add = (label, value) => {
    if (value === null || value === undefined || value === '') return;
    rows.push(`  ${String(label).padEnd(13)}${value}`);
  };

  add('page', meta.page);
  add('when', meta.now || new Date().toISOString());
  add('build', meta.build);
  add('service', meta.service);
  add('transport', state.transport);
  if (state.transportReason) add('why', state.transportReason);
  if (state.lever && state.lever.pinned) add('pinned', state.lever.mode);
  add('relay src', state.relaySource || 'none');
  add('relay probe', state.probe || 'not probed');
  add('signalling', state.signaling);
  if (state.attempt) add('attempt', state.attempt);
  add('ice', state.ice);
  if (state.types && state.types.length) add('candidates', state.types.join(', '));
  if (state.timeoutStage) add('timeout', `at attempt ${state.timeoutStage}`);
  if (state.selectedPair) {
    const p = state.selectedPair;
    add('selected', `${p.local || '?'} -> ${p.remote || '?'}${p.url ? ` via ${p.url}` : ''}`);
  }
  if (state.relayDropped) add('shed', `${state.relayDropped} snapshot frames`);
  for (const [id, peerState] of state.peers || []) add(`peer ${id}`, peerState);
  for (const [id, n] of state.shedding || []) {
    if (n) add(`shed ${id}`, `${n} snapshot frames`);
  }

  return ['phoenix connection diagnostics', ...rows].join('\n');
}

// Published for the classic-script halves of both pages, which cannot import.
if (typeof window !== 'undefined') {
  window.connectionDiagnostics = {
    createClientDiagnostics,
    applyClientDiagEvent,
    clientDiagnosticsLines,
    hostDiagnosticsLines,
    diagnosticsDump,
  };
}

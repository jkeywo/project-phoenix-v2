// What the connection-diagnostics readout says (issue #1113).
//
// This is the surface a person actually uses when a phone will not connect —
// on the viewscreen, on the phone, and in the dump they paste into an issue —
// so what it says is worth asserting rather than eyeballing. It used to be two
// closures inside two HTML files, where nothing could reach it.
//
// The `t` here is the REAL string table (tests/client/setup-strings.js loads
// assets/strings/strings.csv), so a line that names an id the table does not
// carry fails here as well as in scripts/check-strings.mjs.

import { describe, it, expect } from 'vitest';

import { t } from '../../gui/strings.js';
import {
  createClientDiagnostics,
  applyClientDiagEvent,
  clientDiagnosticsLines,
  hostDiagnosticsLines,
  diagnosticsDump,
} from '../../gui/connection-diagnostics.js';
import { readSelectedPair } from '../../gui/connection-manager.js';

/** Every line, joined, so an assertion can ask "does it say this anywhere". */
const clientText = (state) => clientDiagnosticsLines(state, t).join('\n');
const hostText = (state) => hostDiagnosticsLines(state, t).join('\n');

/** A client state that has got as far as a healthy first attempt. */
const connecting = (over = {}) =>
  createClientDiagnostics({
    relayAvailable: true,
    relaySource: 'worker',
    attempt: 1,
    ice: 'checking',
    types: ['host', 'srflx'],
    ...over,
  });

describe('the relay configuration line', () => {
  it('warns when there is no relay at all', () => {
    // The CGNAT/hotspot case: without TURN the connection will simply fail, and
    // the remedy is the credential worker (docs/delivery-checklist.md §3).
    const lines = clientDiagnosticsLines(createClientDiagnostics({ relayAvailable: false }), t);
    expect(lines[0]).toBe(t('client.diag_no_relay'));
  });

  it('names the free shared fallback as a degraded state, not a healthy one', () => {
    // relaySource 'openrelay' means the credential worker was unreachable —
    // which is the 2026-08 outage exactly, and it must never read as fine.
    expect(clientText(connecting({ relaySource: 'openrelay' })))
      .toContain(t('client.diag_relay_fallback'));
    expect(clientText(connecting({ relaySource: 'openrelay', probe: 'reachable' })))
      .toContain(t('client.diag_relay_fallback_ok'));
  });

  it('reports a relay that is configured but not allocatable from here', () => {
    expect(clientText(connecting({ probe: 'unreachable' })))
      .toContain(t('client.diag_relay_fail'));
  });

  it('says nothing about a healthy worker relay that has not been probed', () => {
    // Silence is the readout's healthy state; a line per healthy thing is how a
    // readout stops being read.
    expect(clientDiagnosticsLines(createClientDiagnostics({
      relayAvailable: true, relaySource: 'worker',
    }), t)).toEqual([]);
  });
});

describe('the transport rung', () => {
  it('says nothing while the ordinary direct path is in use', () => {
    expect(clientText(connecting())).not.toContain(t('client.diag_ws_relay'));
  });

  it('says so, actionably, once the service is carrying the game', () => {
    const state = connecting();
    applyClientDiagEvent(state, { event: 'transport', transport: 'ws-relay', reason: 'direct-exhausted' });
    expect(clientText(state)).toContain(t('client.diag_ws_relay'));
  });

  it('names a pinned transport, so a failure is not blamed on the network', () => {
    const state = connecting();
    applyClientDiagEvent(state, { event: 'transport', transport: 'direct', mode: 'turn', pinned: true });
    expect(clientText(state)).toContain(t('client.diag_transport_pinned', { mode: 'turn' }));
  });
});

describe('the per-attempt stage', () => {
  it('reports the attempt, the ICE state and the candidate types gathered', () => {
    expect(clientText(connecting())).toContain(t('client.diag_stage', {
      attempt: 1, ice: 'checking', types: 'host, srflx',
    }));
  });

  it('says which stage a timeout happened at', () => {
    const state = connecting();
    applyClientDiagEvent(state, { event: 'timeout', attempt: 2 });
    expect(state.ice).toBe('timeout');
    expect(state.timeoutStage).toBe(2);
    expect(clientText(state)).toContain('timeout');
  });

  it('forgets the last attempt when a new one starts', () => {
    // A stale ICE state or candidate list left on screen would have the readout
    // describing an attempt that is already over.
    const state = connecting({ selectedPair: { local: 'host', remote: 'host' } });
    applyClientDiagEvent(state, { event: 'attempt', attempt: 2 });
    expect(state).toMatchObject({ attempt: 2, ice: '', types: [], selectedPair: null });
  });

  it('names the route ICE actually chose, and which relay carried it', () => {
    // The question "candidates: host, srflx, relay" cannot answer: those were
    // OFFERED. On a hotspot, whether the selected pair is relayed is the
    // difference between a working network and a working TURN worker.
    const state = connecting();
    applyClientDiagEvent(state, {
      event: 'selected-pair',
      pair: { local: 'relay', remote: 'srflx', protocol: 'tcp', url: 'turn:relay.example:443' },
    });
    expect(clientText(state)).toContain(t('client.diag_selected', {
      local: 'relay', remote: 'srflx', relay: 'turn:relay.example:443',
    }));
  });

  it('shows the placeholder rather than an empty gap when nothing was gathered', () => {
    expect(clientText(connecting({ types: [] }))).toContain(t('client.diag_none'));
  });
});

describe('a relay that is shedding', () => {
  it('says how much has been dropped, and that commands still get through', () => {
    const state = connecting();
    applyClientDiagEvent(state, { event: 'relay-degraded', dropped: 4, from: 'service' });
    expect(clientText(state)).toContain(t('client.diag_relay_shedding', { n: 4 }));
  });

  it('keeps the two ends’ counts apart and adds them', () => {
    // They measure DIFFERENT queues: `client` is what this device shed against
    // its own send buffer on the way up, `service` what the service shed on the
    // way down. Both are running totals, so the old Math.max fold discarded the
    // smaller of two genuinely separate losses.
    const state = connecting();
    applyClientDiagEvent(state, { event: 'relay-degraded', dropped: 4, from: 'client' });
    applyClientDiagEvent(state, { event: 'relay-degraded', dropped: 2, from: 'service' });
    expect(state.relayDropped).toBe(6);
  });

  it('does not double-count one end reporting its running total again', () => {
    // The half the Math.max fold got right, and the half a naive sum would get
    // wrong: consecutive reports from the SAME end are the same frames counted
    // again, not new ones.
    const state = connecting();
    for (const dropped of [1, 2, 3, 12]) {
      applyClientDiagEvent(state, { event: 'relay-degraded', dropped, from: 'service' });
    }
    expect(state.relayDropped).toBe(12);
  });

  it('does not pin at 1 while the service keeps reporting', () => {
    // The bug this fold existed to have: the service used to send a per-enqueue
    // delta, which is 1 essentially always once a queue is at its bound, so the
    // readout said "Dropping display updates (1)" for the whole mission.
    const state = connecting();
    for (let n = 1; n <= 200; n += 1) {
      applyClientDiagEvent(state, { event: 'relay-degraded', dropped: n, from: 'service' });
    }
    expect(state.relayDropped).toBe(200);
    expect(clientText(state)).toContain(t('client.diag_relay_shedding', { n: 200 }));
  });
});

describe('the host readout', () => {
  const host = (over = {}) => ({
    relayAvailable: true,
    relaySource: 'worker',
    probe: null,
    lever: { mode: 'auto', pinned: false },
    reregister: true,
    fault: null,
    retrying: false,
    peers: [],
    ...over,
  });

  it('explains a missing join code differently depending on whether one is coming', () => {
    // The operator's next move differs: wait for a new code, or wait for the
    // service. Two sentences, not one.
    expect(hostText(host({ fault: 'unreachable', retrying: true })))
      .toContain(t('server.join.rendezvous_reregistering'));
    expect(hostText(host({ fault: 'forbidden-role', retrying: false })))
      .toContain(t('server.join.rendezvous_lost'));
  });

  it('promises no retry when re-registration is switched off', () => {
    // The sentence on screen must not promise a new code from a host that is
    // not going to ask for one.
    expect(hostText(host({ fault: 'unreachable', retrying: true, reregister: false })))
      .toContain(t('server.join.rendezvous_lost'));
  });

  it('names a joiner stuck mid-ICE, which is the only sign it is trying', () => {
    expect(hostText(host({ peers: [['ab12cd34', 'checking']] })))
      .toContain(t('server.client_ice_state', { id: 'ab12cd34', state: 'checking' }));
  });

  it('gives a relayed crew member its own sentence rather than an ICE state', () => {
    // 'ws-relay' is not one of WebRTC's five states, and rendering it through
    // the ICE line would read as "connection ws-relay", which says nothing
    // about what the operator is looking at.
    const text = hostText(host({ peers: [['ab12cd34', 'ws-relay']] }));
    expect(text).toContain(t('server.client_ws_relay', { id: 'ab12cd34' }));
    expect(text).not.toContain(t('server.client_ice_state', { id: 'ab12cd34', state: 'ws-relay' }));
  });

  it('shows a shedding relayed peer BOTH facts, not one instead of the other', () => {
    // The shed count used to be written into the same map as the transport
    // marker, as a fabricated `relay-shedding-3` state. That is not one of
    // ICE's five, so the row stopped rendering "carried by the join service" —
    // the exact line docs/acceptance/1113-networks.md tells a tester to look
    // for — and printed the raw token through the ICE sentence instead.
    const text = hostText(host({
      peers: [['ab12cd34', 'ws-relay']],
      shedding: [['ab12cd34', 3]],
    }));
    expect(text).toContain(t('server.client_ws_relay', { id: 'ab12cd34' }));
    expect(text).toContain(t('server.client_relay_shedding', { id: 'ab12cd34', n: 3 }));
    expect(text).not.toContain('relay-shedding');
  });

  it('says nothing about a peer that has shed nothing', () => {
    expect(hostDiagnosticsLines(host({ shedding: [['ab12cd34', 0]] }), t)).toEqual([]);
  });

  it('is silent on a healthy host with nobody in trouble', () => {
    expect(hostDiagnosticsLines(host(), t)).toEqual([]);
  });
});

describe('the copy-pasteable dump', () => {
  it('carries what the acceptance kit asks a tester to report', () => {
    const state = connecting({ relaySource: 'openrelay', probe: 'reachable' });
    applyClientDiagEvent(state, { event: 'transport', transport: 'ws-relay', reason: 'direct-exhausted' });
    applyClientDiagEvent(state, { event: 'timeout', attempt: 3 });
    applyClientDiagEvent(state, { event: 'relay-degraded', dropped: 7 });
    const dump = diagnosticsDump(state, {
      page: 'client',
      now: '2026-08-28T05:00:00.000Z',
      service: 'https://phoenix-rendezvous.example',
      build: '1/phoenix-base/3',
    });
    for (const expected of [
      'page         client',
      'service      https://phoenix-rendezvous.example',
      'build        1/phoenix-base/3',
      'transport    ws-relay',
      'why          direct-exhausted',
      'relay src    openrelay',
      'relay probe  reachable',
      'candidates   host, srflx',
      'timeout      at attempt 3',
      'shed         7 snapshot frames',
    ]) {
      expect(dump).toContain(expected);
    }
    // And NOT a network row. Neither page has a field for one and nothing can
    // invent it, so the row was always silently omitted while the acceptance
    // kit's worked example showed it — documenting output the product cannot
    // produce. The kit now asks the tester to write the network above the
    // pasted block, which is where a human sentence belongs.
    expect(dump).not.toContain('network');
  });

  it('never carries a join code, a token or a full peer id', () => {
    // It is pasted into a public issue thread by somebody standing in a car
    // park. The suffix is the private client code; a dump that leaked it would
    // be a dump nobody could safely paste.
    const dump = diagnosticsDump(
      { ...connecting(), peers: [['ab12cd34', 'ws-relay']] },
      { page: 'host', service: 'https://phoenix-rendezvous.example' },
    );
    expect(dump).not.toMatch(/[A-Z]{5}/);
    expect(dump).toContain('peer ab12cd34');
  });

  it('leaves out what it does not know rather than printing blanks', () => {
    const dump = diagnosticsDump(createClientDiagnostics(), { page: 'client', now: 'T' });
    expect(dump).not.toContain('build');
    expect(dump).not.toContain('attempt');
    expect(dump).toContain('relay src    none');
  });

  it('agrees with the readout about which transport is in use', () => {
    // The whole reason the state lives in one place: a dump that said 'direct'
    // while the screen said the service was carrying the game would send
    // whoever read it after the wrong bug.
    const state = connecting();
    applyClientDiagEvent(state, { event: 'transport', transport: 'ws-relay' });
    expect(clientText(state)).toContain(t('client.diag_ws_relay'));
    expect(diagnosticsDump(state, {})).toContain('transport    ws-relay');
  });
});

describe('reading the selected candidate pair', () => {
  /** A `getStats()` result shaped like the browsers' RTCStatsReport. */
  const report = (entries) => new Map(entries.map((e) => [e.id, e]));

  it('follows the nominated succeeded pair to its candidates', async () => {
    const pc = {
      getStats: async () => report([
        { id: 'p1', type: 'candidate-pair', state: 'failed', nominated: false, localCandidateId: 'l0', remoteCandidateId: 'r0' },
        { id: 'p2', type: 'candidate-pair', state: 'succeeded', nominated: true, localCandidateId: 'l1', remoteCandidateId: 'r1' },
        { id: 'l1', type: 'local-candidate', candidateType: 'relay', relayProtocol: 'tcp', url: 'turn:relay.example:443' },
        { id: 'r1', type: 'remote-candidate', candidateType: 'srflx' },
      ]),
    };
    expect(await readSelectedPair(pc)).toEqual({
      local: 'relay', remote: 'srflx', protocol: 'tcp', url: 'turn:relay.example:443',
    });
  });

  it('accepts Firefox’s spelling of the same fact', async () => {
    const pc = {
      getStats: async () => report([
        { id: 'p1', type: 'candidate-pair', selected: true, localCandidateId: 'l1', remoteCandidateId: 'r1' },
        { id: 'l1', type: 'local-candidate', candidateType: 'host', protocol: 'udp' },
        { id: 'r1', type: 'remote-candidate', candidateType: 'host' },
      ]),
    };
    expect(await readSelectedPair(pc)).toMatchObject({ local: 'host', protocol: 'udp' });
  });

  it('answers null rather than throwing when there are no stats to read', async () => {
    // The smoke suite's fake peer connection returns an empty Map, a closed
    // connection can reject, and a native/relayed link has no getStats at all.
    // A diagnostics line is never worth a thrown error.
    expect(await readSelectedPair(null)).toBeNull();
    expect(await readSelectedPair({})).toBeNull();
    expect(await readSelectedPair({ getStats: async () => new Map() })).toBeNull();
    expect(await readSelectedPair({ getStats: async () => { throw new Error('closed'); } })).toBeNull();
  });
});

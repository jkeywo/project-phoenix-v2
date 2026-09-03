// The transport pins (issue #1113).
//
// gui/transport-levers.js is what lets a test — and a human working through
// docs/acceptance/1113-networks.md on a real network — get onto a rung of the
// transport ladder that a healthy network would never reach, because the first
// rung always wins. Pure, so what a lever MEANS is asserted here once rather
// than inferred from behaviour in each of the places that reads one.

import { describe, it, expect } from 'vitest';

import {
  transportLeversFromLocation,
  defaultTransportLevers,
  TRANSPORT_AUTO,
  TRANSPORT_DIRECT,
  TRANSPORT_TURN,
  TRANSPORT_WS_RELAY,
} from '../../gui/transport-levers.js';

describe('the default', () => {
  it('tries every rung, in order', () => {
    for (const search of ['', '?', '?scenario=combat_test']) {
      expect(transportLeversFromLocation(search)).toEqual({
        mode: TRANSPORT_AUTO,
        iceTransportPolicy: 'all',
        useIceServers: true,
        wsRelay: 'auto',
        pinned: false,
      });
    }
    expect(defaultTransportLevers().mode).toBe(TRANSPORT_AUTO);
  });

  it('survives a missing search string', () => {
    // Both pages read `location.search`, which a `file://` load or a page
    // opened with no query leaves as an empty string — and a joiner built by a
    // caller that passed nothing at all must still get a working ladder.
    expect(transportLeversFromLocation(undefined).mode).toBe(TRANSPORT_AUTO);
    expect(transportLeversFromLocation(null).pinned).toBe(false);
  });
});

describe('?forceRelay — the shorthand the acceptance kit uses', () => {
  it('accepts every spelling a human types on the end of a join link', () => {
    for (const search of ['?forceRelay', '?forceRelay=', '?forceRelay=1',
      '?forceRelay=true', '?forceRelay=on', '?forceRelay=YES']) {
      const levers = transportLeversFromLocation(search);
      expect(levers.mode, search).toBe(TRANSPORT_TURN);
      expect(levers.iceTransportPolicy, search).toBe('relay');
    }
  });

  it('turns off the WebSocket fallback as well as the direct candidates', () => {
    // The whole point of pinning TURN is to find out whether TURN works. A
    // fallback that quietly rescued the session would report success for a
    // relay that is in fact broken — which is the 2026-08 TURN outage's exact
    // failure mode, arrived at from the other direction.
    expect(transportLeversFromLocation('?forceRelay=1').wsRelay).toBe('off');
  });

  it('is ignored when it is off, rather than meaning something', () => {
    for (const search of ['?forceRelay=0', '?forceRelay=off', '?forceRelay=no']) {
      expect(transportLeversFromLocation(search).mode, search).toBe(TRANSPORT_AUTO);
    }
  });
});

describe('?transport — the full lever', () => {
  it('pins direct WebRTC with no relay of either kind', () => {
    expect(transportLeversFromLocation('?transport=direct')).toEqual({
      mode: TRANSPORT_DIRECT,
      iceTransportPolicy: 'all',
      useIceServers: false,
      wsRelay: 'off',
      pinned: true,
    });
  });

  it('makes the direct pin real by withholding the relay servers', () => {
    // The title above used to be a claim the implementation did not keep:
    // direct gave `iceTransportPolicy: 'all'`, which is exactly what auto
    // gives, so ICE was free to select a relayed pair and the only thing the
    // pin actually did was switch off the WebSocket fallback. There is no
    // 'no-relay' policy value, so the only honest spelling of "no relay in
    // this path" is to hand the peer connection no relay servers at all.
    const direct = transportLeversFromLocation('?transport=direct');
    const auto = transportLeversFromLocation('');
    expect(direct.useIceServers).toBe(false);
    expect(auto.useIceServers).toBe(true);
    // …and it is genuinely narrower than auto in the ICE plane, not only in
    // the fallback one — which is what its name promises a field tester.
    expect(direct.useIceServers).not.toBe(auto.useIceServers);
    // The TURN pin is its mirror image: relay candidates ONLY, servers kept.
    const turn = transportLeversFromLocation('?transport=turn');
    expect(turn.useIceServers).toBe(true);
    expect(turn.iceTransportPolicy).toBe('relay');
  });

  it('pins the WebSocket relay, skipping WebRTC entirely', () => {
    expect(transportLeversFromLocation('?transport=ws-relay')).toEqual({
      mode: TRANSPORT_WS_RELAY,
      iceTransportPolicy: 'all',
      useIceServers: true,
      wsRelay: 'only',
      pinned: true,
    });
  });

  it('agrees with the shorthand about what TURN-only means', () => {
    expect(transportLeversFromLocation('?transport=turn'))
      .toEqual(transportLeversFromLocation('?forceRelay=1'));
  });

  it('ignores a value it does not implement rather than guessing', () => {
    // An old or mistyped link must open the game, not pin it to a mode that
    // does not exist — the same posture `?rendezvous=on` gets in
    // gui/rendezvous-transport.js.
    for (const search of ['?transport=quic', '?transport=', '?transport=RELAY ']) {
      expect(transportLeversFromLocation(search).mode, search).toBe(TRANSPORT_AUTO);
    }
  });

  it('reads a mode name case-insensitively and trimmed', () => {
    expect(transportLeversFromLocation('?transport=%20WS-Relay%20').mode)
      .toBe(TRANSPORT_WS_RELAY);
  });

  it('lets an explicit mode win over the shorthand', () => {
    // A link carrying both has one obvious reading rather than an ordering
    // puzzle: the specific lever beats the shorthand, and an explicit `auto`
    // is the default anyway, so the shorthand still applies there.
    expect(transportLeversFromLocation('?transport=ws-relay&forceRelay=1').mode)
      .toBe(TRANSPORT_WS_RELAY);
    expect(transportLeversFromLocation('?transport=auto&forceRelay=1').mode)
      .toBe(TRANSPORT_TURN);
  });
});

describe('what a lever can and cannot do', () => {
  it('can only ever make the transport try LESS', () => {
    // The safety argument for shipping a URL lever at all: no value reaches a
    // host, a peer or a payload that a default load could not, so the worst a
    // hostile link can do with one is hand its recipient a worse connection —
    // visibly, because a pinned transport is named in the diagnostics readout.
    const auto = transportLeversFromLocation('');
    for (const search of ['?transport=direct', '?transport=turn', '?transport=ws-relay']) {
      const pinned = transportLeversFromLocation(search);
      const narrower =
        pinned.iceTransportPolicy !== auto.iceTransportPolicy
        || pinned.useIceServers !== auto.useIceServers
        || pinned.wsRelay !== auto.wsRelay;
      expect(narrower, search).toBe(true);
      expect(pinned.pinned, search).toBe(true);
    }
  });
});

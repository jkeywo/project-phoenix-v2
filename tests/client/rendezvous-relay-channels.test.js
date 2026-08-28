// The client's half of the WebSocket game relay (issue #1113).
//
// gui/rendezvous-relay.js hands back objects shaped like `RTCDataChannel`, and
// that shape is the whole mechanism by which the relay does not fork the game
// protocol: gui/rendezvous-transport.js wires them through the same admission
// gate, the same adapter and the same delivery-class routing it wires a real
// channel through, because it cannot tell them apart. These cases pin the
// shape, and the one behaviour that is genuinely the relay's own — keeping the
// lossy class lossy over a transport that is not.

import { describe, it, expect } from 'vitest';

import {
  createRelayChannelPair,
  relayLimitsFromFrame,
  relayPeerStub,
  RELAY_LIMIT_DEFAULTS,
  RELAY_RELIABLE_LABEL,
  RELAY_SNAPSHOT_LABEL,
} from '../../gui/rendezvous-relay.js';

/** A pair plus the frames it put on the wire. */
function pairOn(over = {}) {
  const sent = [];
  const degraded = [];
  const pair = createRelayChannelPair({
    send: (frame) => sent.push(frame),
    onDegraded: (info) => degraded.push(info),
    ...over,
  });
  return { pair, sent, degraded };
}

describe('the channels are DataChannel-shaped', () => {
  it('carries the same two labels a negotiated pair does', () => {
    const { pair } = pairOn();
    expect(pair.reliable.label).toBe(RELAY_RELIABLE_LABEL);
    expect(pair.snapshot.label).toBe(RELAY_SNAPSHOT_LABEL);
    // The host reads `readyState` off whatever it is handed
    // (gui/rendezvous-transport.js's connectionAdapter), so a relayed channel
    // has to speak the same three words a real one does.
    expect(pair.reliable.readyState).toBe('connecting');
    pair.open();
    expect(pair.reliable.readyState).toBe('open');
    pair.close();
    expect(pair.reliable.readyState).toBe('closed');
  });

  it('fires open and close exactly once, as a real channel does', () => {
    const { pair } = pairOn();
    const events = [];
    pair.reliable.onopen = () => events.push('open');
    pair.reliable.onclose = () => events.push('close');
    pair.open();
    pair.open();
    pair.close();
    pair.close();
    expect(events).toEqual(['open', 'close']);
  });

  it('drops a send made before open or after close, rather than throwing', () => {
    // The transport's own `send` is called from console controls and from the
    // host's outbound flush; an exception there would abort delivery to every
    // remaining crew member, which is the reason connectionAdapter guards its
    // own send too.
    const { pair, sent } = pairOn();
    pair.reliable.send('early');
    pair.open();
    pair.reliable.send('{"type":"SetReady"}');
    pair.close();
    pair.reliable.send('late');
    expect(sent).toHaveLength(1);
    expect(sent[0]).toMatchObject({ type: 'relay', class: 'reliable' });
  });

  it('routes an inbound frame to the channel its class names', () => {
    const { pair } = pairOn();
    const reliable = [];
    const snapshot = [];
    pair.reliable.onmessage = (e) => reliable.push(e.data);
    pair.snapshot.onmessage = (e) => snapshot.push(e.data);
    pair.open();
    pair.deliver({ class: 'snapshot', payload: 'S' });
    pair.deliver({ class: 'reliable', payload: 'R' });
    // A frame with no class at all is reliable, not discarded: an unclassed
    // payload from an older service is a message, and losing a command is worse
    // than delivering a snapshot on the reliable channel.
    pair.deliver({ payload: 'U' });
    expect(snapshot).toEqual(['S']);
    expect(reliable).toEqual(['R', 'U']);
  });

  it('addresses a peer only when a host names one', () => {
    // The joiner omits `to` — the service already knows the only destination —
    // while the host must name which relayed crew member it means.
    const joiner = pairOn();
    joiner.pair.open();
    joiner.pair.reliable.send('x');
    expect(joiner.sent[0].to).toBeUndefined();

    const host = pairOn({ to: 'peer-7' });
    host.pair.open();
    host.pair.reliable.send('x');
    expect(host.sent[0].to).toBe('peer-7');
  });
});

describe('the lossy class stays lossy', () => {
  it('sheds a snapshot frame once the socket has a backlog, and says so', () => {
    let backlog = 0;
    const { pair, sent, degraded } = pairOn({
      bufferedAmount: () => backlog,
      limits: { maxFrameBytes: 1024, maxSendBufferBytes: 100 },
    });
    pair.open();

    pair.snapshot.send('fresh');
    expect(sent).toHaveLength(1);

    backlog = 500;
    pair.snapshot.send('stale');
    pair.snapshot.send('stale');
    // Queuing these behind the backlog is the head-of-line blocking the
    // snapshot class exists to avoid, and the next tick supersedes them.
    expect(sent).toHaveLength(1);
    expect(degraded).toEqual([
      { dropped: 1, reason: 'send-buffer' },
      { dropped: 2, reason: 'send-buffer' },
    ]);
    expect(pair.droppedSnapshots).toBe(2);
  });

  it('never sheds a reliable frame, whatever the backlog', () => {
    // A WebSocket buffers what it is given; a dropped command would break the
    // guarantee the game is written against.
    const { pair, sent, degraded } = pairOn({
      bufferedAmount: () => 10_000_000,
      limits: { maxFrameBytes: 1024, maxSendBufferBytes: 100 },
    });
    pair.open();
    pair.reliable.send('{"type":"SetRedAlert"}');
    pair.reliable.send('{"type":"SetReady"}');
    expect(sent).toHaveLength(2);
    expect(degraded).toEqual([]);
  });

  it('does not shed at all when the transport cannot report a backlog', () => {
    // The default `bufferedAmount` is 0 rather than a fabricated number: a
    // transport with no backpressure signal should send, not guess.
    const { pair, sent } = pairOn({ limits: { maxFrameBytes: 1024, maxSendBufferBytes: 0 } });
    pair.open();
    pair.snapshot.send('a');
    pair.snapshot.send('b');
    expect(sent).toHaveLength(2);
  });

  it('refuses an oversized frame locally rather than being cut off at the ceiling', () => {
    const { pair, sent } = pairOn({ limits: { maxFrameBytes: 8, maxSendBufferBytes: 1024 } });
    pair.open();
    pair.reliable.send('123456789');
    expect(sent).toEqual([]);
    pair.reliable.send('12345678');
    expect(sent).toHaveLength(1);
  });

  it('measures that ceiling in bytes, matching the service', () => {
    // Four euro signs are 4 UTF-16 units and 12 bytes. A client that measured
    // in units would send a frame the service then refused, and the operator
    // would see a link fail for a payload the client thought was fine.
    const { pair, sent } = pairOn({ limits: { maxFrameBytes: 8, maxSendBufferBytes: 1024 } });
    pair.open();
    pair.reliable.send('€€€€');
    expect(sent).toEqual([]);
  });
});

describe('the advertised limits', () => {
  it('takes the service’s numbers over its own defaults', () => {
    const limits = relayLimitsFromFrame({
      max_frame_bytes: 4096,
      max_send_buffer_bytes: 1024,
    });
    expect(limits).toEqual({ maxFrameBytes: 4096, maxSendBufferBytes: 1024 });
  });

  it('falls back per field, so a partial advertisement is still usable', () => {
    expect(relayLimitsFromFrame({ max_frame_bytes: 4096 })).toEqual({
      maxFrameBytes: 4096,
      maxSendBufferBytes: RELAY_LIMIT_DEFAULTS.maxSendBufferBytes,
    });
    expect(relayLimitsFromFrame(undefined)).toEqual(RELAY_LIMIT_DEFAULTS);
  });

  it('adopts a later advertisement without rebuilding the pair', () => {
    const { pair, sent } = pairOn({ limits: { maxFrameBytes: 4, maxSendBufferBytes: 1024 } });
    pair.open();
    pair.reliable.send('12345');
    expect(sent).toEqual([]);
    pair.applyLimits({ maxFrameBytes: 64, maxSendBufferBytes: 1024 });
    pair.reliable.send('12345');
    expect(sent).toHaveLength(1);
  });
});

describe('the peer-connection stand-in', () => {
  it('answers the questions a caller asks of a real peer connection', async () => {
    // connectionAdapter closes one, and server.html reads its state for
    // diagnostics; handing those sites a null would make each of them grow a
    // guard for a case that is not exceptional.
    let closed = false;
    const pc = relayPeerStub(() => { closed = true; });
    expect(await pc.getStats()).toEqual(new Map());
    pc.close();
    expect(closed).toBe(true);
  });

  it('does not claim an ICE result nothing negotiated', () => {
    // Borrowing 'connected' here would put a WebRTC verdict on a readout for a
    // link that never ran ICE at all.
    expect(relayPeerStub().iceConnectionState).toBe('relayed');
  });
});

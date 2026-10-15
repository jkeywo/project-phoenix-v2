import {describe, it, expect, vi} from 'vitest';
import {sendSnapshotOnly} from '../../packages/transport/src/snapshot-only.js';
import {createRelayChannelPair} from '../../packages/transport/src/rendezvous-relay.js';

const bounds = {maxBytes: 20 * 1024, maxBufferedBytes: 8192};
describe('opaque Snapshot-only send', () => {
  it('measures actual encoded bytes and actual backlog without another channel', () => {
    const channel = {readyState: 'open', bufferedAmount: 0, send: vi.fn()};
    expect(sendSnapshotOnly(channel, 'ordinary', bounds)).toBe(true);
    expect(sendSnapshotOnly(channel, 'é'.repeat(11000), bounds)).toBe(false);
    channel.bufferedAmount = 8188;
    expect(sendSnapshotOnly(channel, 'would exceed actual buffer bound', bounds)).toBe(false);
    channel.bufferedAmount = 8193;
    expect(sendSnapshotOnly(channel, 'late', bounds)).toBe(false);
    channel.bufferedAmount = undefined;
    expect(sendSnapshotOnly(channel, 'unknown backlog', bounds)).toBe(false);
    expect(channel.send.mock.calls).toEqual([['ordinary']]);
  });
  it('drops unavailable, malformed and throwing sends without retry', () => {
    const send = vi.fn(() => { throw new Error('gone'); });
    expect(sendSnapshotOnly(null, 'missing', bounds)).toBe(false);
    expect(sendSnapshotOnly({readyState: 'closed', bufferedAmount: 0, send}, 'closed', bounds)).toBe(false);
    expect(sendSnapshotOnly({readyState: 'open', bufferedAmount: 0, send}, 'throws', bounds)).toBe(false);
    expect(sendSnapshotOnly({readyState: 'open', bufferedAmount: 0, send}, 'unbounded')).toBe(false);
    expect(send).toHaveBeenCalledTimes(1);
  });
  it('uses the relay socket backlog and preserves ordinary Reliable commands', () => {
    let backlog = 0;
    const sent = [];
    const pair = createRelayChannelPair({send: frame => sent.push(frame), bufferedAmount: () => backlog});
    pair.open();
    expect(sendSnapshotOnly(pair.snapshot, 'first', bounds)).toBe(true);
    backlog = 8193;
    expect(sendSnapshotOnly(pair.snapshot, 'shed', bounds)).toBe(false);
    pair.reliable.send('command');
    expect(sent.map(frame => [frame.class, frame.payload])).toEqual([
      ['snapshot', 'first'], ['reliable', 'command'],
    ]);
    backlog = 0;
    const oversized = 'x'.repeat(70000);
    // The relay's own negotiated ceiling is independently enforced.
    expect(sendSnapshotOnly(pair.snapshot, oversized, {maxBytes: 80000, maxBufferedBytes: 8192})).toBe(false);
    expect(pair.reliable.readyState).toBe('open');
  });
  it('checks actual relay readiness and complete wire envelope bytes', () => {
    let writable = true;
    const send = vi.fn(() => true);
    const measureFrame = body => new TextEncoder().encode(JSON.stringify({v: 1, ...body})).byteLength;
    const pair = createRelayChannelPair({send, writable: () => writable,
      bufferedAmount: () => 10, measureFrame, to: 'actual-peer'});
    pair.open();
    const packet = '{"type":"opaque"}';
    const wireBytes = pair.snapshot.sendByteLength(packet);
    expect(wireBytes).toBeGreaterThan(packet.length);
    expect(sendSnapshotOnly(pair.snapshot, packet, {maxBytes: 100, maxBufferedBytes: wireBytes + 9})).toBe(false);
    expect(sendSnapshotOnly(pair.snapshot, packet, {maxBytes: 100, maxBufferedBytes: wireBytes + 10})).toBe(true);
    writable = false;
    expect(sendSnapshotOnly(pair.snapshot, packet, bounds)).toBe(false);
    expect(pair.snapshot.send(packet)).toBe(false);
    expect(send).toHaveBeenCalledTimes(1);
    const failed = createRelayChannelPair({send: () => false}); failed.open();
    expect(sendSnapshotOnly(failed.snapshot, packet, bounds)).toBe(false);
  });
});

import { afterEach, describe, expect, it, vi } from 'vitest';
import { impairDataChannel } from '../../scripts/fleet-channel-impairment.mjs';

function channel(label) {
  const listeners = new Map();
  const instance = { label, readyState: 'open', sent: [],
    send(value) { if (this.readyState !== 'open') throw new Error('closed'); this.sent.push(value); },
    addEventListener(event, callback) { listeners.set(event, callback); },
    close() { this.readyState = 'closed'; listeners.get('close')?.(); } };
  return instance;
}
const profile = { delayMs: 20, lossPercent: 50, seed: 1530 };
const clock = { now: () => Date.now() };
afterEach(() => vi.useRealTimers());
describe('real DataChannel boundary impairment', () => {
  it('retains every reliable frame in order and records actual scheduled delay', async () => {
    vi.useFakeTimers(); const counters = {}, link = channel('reliable');
    impairDataChannel(link, profile, counters, clock);
    link.send('a'); link.send('b');
    expect(link.sent).toEqual([]);
    await vi.advanceTimersByTimeAsync(20);
    expect(link.sent).toEqual(['a', 'b']);
    expect(counters.reliable).toMatchObject({ seen: 2, written: 2, dropped: 0, delay: { count: 2, min: 20, max: 20 } });
    expect(counters.pending).toBe(0);
  });
  it('makes snapshot loss reproducible while never applying it to reliable traffic', () => {
    // Compare seeded loss with the same clock; real elapsed send time is not seeded.
    vi.useFakeTimers();
    const run = label => { const link = channel(label), counters = {}; impairDataChannel(link, { ...profile, delayMs: 0 }, counters, clock); for (let i = 0; i < 100; i++) link.send(i); return { sent: link.sent, counters }; };
    expect(run('snapshot')).toEqual(run('snapshot'));
    expect(run('snapshot').counters.snapshot.dropped).toBeGreaterThan(0);
    expect(run('snapshot').counters.snapshot.written).toBeGreaterThan(0);
    expect(run('reliable').sent).toHaveLength(100);
  });
  it('cancels queued frames on close without later writing them', async () => {
    vi.useFakeTimers(); const counters = {}, link = channel('reliable'); impairDataChannel(link, profile, counters, clock);
    link.send('a'); link.close(); await vi.advanceTimersByTimeAsync(100);
    expect(link.sent).toEqual([]); expect(counters.reliable.cancelled).toBe(1); expect(counters.pending).toBe(0);
  });
  it('bounds the queue and closes instead of silently losing reliable data', () => {
    vi.useFakeTimers(); const counters = {}, link = channel('reliable'); impairDataChannel(link, profile, counters, clock);
    for (let i = 0; i < 513; i++) link.send(i);
    expect(link.readyState).toBe('closed'); expect(counters.overflows).toBe(1); expect(counters.pending).toBe(0); expect(counters.reliable.cancelled).toBe(513);
  });
  it('leaves non-game diagnostic channels untouched', () => {
    const counters = {}, link = channel('turn-probe'); impairDataChannel(link, profile, counters, clock);
    link.send('probe'); expect(link.sent).toEqual(['probe']); expect(counters).toEqual({});
  });
});

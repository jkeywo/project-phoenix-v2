// Issue #1169 — client-side console input-to-feedback measurement.
//
// The meter lives in gui/console-latency.js precisely so it can be driven here
// without a browser, a phone or a WASM bundle: it is a small state machine over
// an injected clock. These tests pin the properties the measurement's honesty
// rests on — one clock, bounded everything, and nothing invented.

import { describe, it, expect } from 'vitest';
import {
  ConsoleLatencyMeter,
  consoleLatencyMessage,
  isConsoleLatencyEnabled,
  nowMs,
  MAX_BATCH,
} from '../../gui/console-latency.js';

/** A meter over a clock the test advances by hand. */
function meterAt(surface = 'PhoneConsole', opts = {}) {
  const clock = { t: 1000 };
  const meter = new ConsoleLatencyMeter({ surface, now: () => clock.t, ...opts });
  meter.setEnabled(true);
  return { meter, clock };
}

describe('nowMs', () => {
  it('is epoch-relative, so two documents on one device share an axis', () => {
    const t = nowMs();
    expect(Number.isFinite(t)).toBe(true);
    // A bare performance.now() is a few milliseconds since this document loaded;
    // an epoch-relative reading is decades. The distinction is the whole point:
    // a console iframe's timeOrigin is not its shell's.
    expect(t).toBeGreaterThan(1e12);
  });

  it('is monotonically non-decreasing', () => {
    const a = nowMs();
    const b = nowMs();
    expect(b).toBeGreaterThanOrEqual(a);
  });
});

describe('isConsoleLatencyEnabled', () => {
  it('reads the host flag out of the DebugState fold', () => {
    expect(isConsoleLatencyEnabled({ flags: { ConsoleLatency: true } })).toBe(true);
    expect(isConsoleLatencyEnabled({ flags: { ConsoleLatency: false } })).toBe(false);
  });

  it('never guesses: no report from the host reads as OFF', () => {
    expect(isConsoleLatencyEnabled(null)).toBe(false);
    expect(isConsoleLatencyEnabled(undefined)).toBe(false);
    expect(isConsoleLatencyEnabled({})).toBe(false);
    expect(isConsoleLatencyEnabled({ flags: {} })).toBe(false);
  });
});

describe('ConsoleLatencyMeter — the two segments', () => {
  it('splits client-local work from the round trip', () => {
    const { meter, clock } = meterAt();
    // The console stamped its input event 3 ms before the shell dispatched.
    meter.noteDispatch('fire_phaser', 'tactical', clock.t - 3);
    clock.t += 47;
    meter.noteAck('tactical');

    const [sample] = meter.drain();
    expect(sample).toEqual({
      action: 'fire_phaser',
      surface: 'PhoneConsole',
      input_to_send_ms: 3,
      send_to_ack_ms: 47,
    });
  });

  it('reports zero client-local time rather than guessing when nothing stamped', () => {
    const { meter, clock } = meterAt();
    // A dispatch from something that is not a console control (the host page's
    // own scenario picker) carries no `__input_ms`.
    meter.noteDispatch('select_scenario', 'tactical', undefined);
    clock.t += 20;
    meter.noteAck('tactical');

    const [sample] = meter.drain();
    expect(sample.input_to_send_ms).toBe(0);
    expect(sample.send_to_ack_ms).toBe(20);
  });

  it('resolves every action pending on the surface that refreshed', () => {
    const { meter, clock } = meterAt();
    meter.noteDispatch('fire_phaser', 'tactical', clock.t);
    clock.t += 5;
    meter.noteDispatch('set_target', 'tactical', clock.t);
    clock.t += 30;
    meter.noteAck('tactical');

    const samples = meter.drain();
    expect(samples.map((s) => s.action)).toEqual(['fire_phaser', 'set_target']);
    expect(samples.map((s) => s.send_to_ack_ms)).toEqual([35, 30]);
  });

  it('a refresh of a DIFFERENT console acknowledges nothing', () => {
    const { meter, clock } = meterAt();
    meter.noteDispatch('fire_phaser', 'tactical', clock.t);
    clock.t += 10;
    meter.noteAck('repair');

    expect(meter.drain()).toEqual([]);
    expect(meter.pendingCount()).toBe(1);
  });

  it('carries the surface it was constructed for', () => {
    const { meter, clock } = meterAt('BrowserHost');
    meter.noteDispatch('fire_phaser', 'host', clock.t);
    meter.noteAck('host');
    expect(meter.drain()[0].surface).toBe('BrowserHost');
  });
});

describe('ConsoleLatencyMeter — the host owns the switch', () => {
  it('measures nothing until enabled', () => {
    const clock = { t: 0 };
    const meter = new ConsoleLatencyMeter({ surface: 'PhoneConsole', now: () => clock.t });
    expect(meter.enabled).toBe(false);
    meter.noteDispatch('fire_phaser', 'tactical', 0);
    clock.t += 50;
    meter.noteAck('tactical');
    expect(meter.drain()).toEqual([]);
    expect(meter.pendingCount()).toBe(0);
  });

  /// Turning off mid-flight discards everything: an ack landing after a
  /// re-enable would carry a `send_to_ack` spanning the gap, and a fabricated
  /// outlier is worse than a missing sample.
  it('discards work in flight when switched off', () => {
    const { meter, clock } = meterAt();
    meter.noteDispatch('fire_phaser', 'tactical', clock.t);
    meter.setEnabled(false);
    meter.setEnabled(true);
    clock.t += 5000;
    meter.noteAck('tactical');
    expect(meter.drain()).toEqual([]);
  });
});

describe('ConsoleLatencyMeter — everything is bounded', () => {
  it('an action that is never acknowledged expires rather than becoming an outlier', () => {
    const { meter, clock } = meterAt('PhoneConsole', { expiryMs: 1000 });
    meter.noteDispatch('fire_phaser', 'tactical', clock.t);
    clock.t += 1500; // no refresh arrived in time
    meter.noteAck('tactical');
    expect(meter.drain()).toEqual([]);
    expect(meter.pendingCount()).toBe(0);
  });

  it('caps unanswered actions per surface, dropping the oldest', () => {
    const { meter, clock } = meterAt('PhoneConsole', { maxPending: 2 });
    for (const action of ['a', 'b', 'c']) {
      meter.noteDispatch(action, 'tactical', clock.t);
      clock.t += 1;
    }
    expect(meter.pendingCount()).toBe(2);
    meter.noteAck('tactical');
    expect(meter.drain().map((s) => s.action)).toEqual(['b', 'c']);
  });

  it('caps the outgoing batch at the wire limit, keeping the most recent', () => {
    const { meter, clock } = meterAt('PhoneConsole', { maxPending: 200, maxBatch: 3 });
    for (let i = 0; i < 10; i += 1) {
      meter.noteDispatch(`a${i}`, 'tactical', clock.t);
    }
    meter.noteAck('tactical');
    const samples = meter.drain();
    expect(samples).toHaveLength(3);
    expect(samples.map((s) => s.action)).toEqual(['a7', 'a8', 'a9']);
  });

  it('drain empties the meter', () => {
    const { meter, clock } = meterAt();
    meter.noteDispatch('fire_phaser', 'tactical', clock.t);
    meter.noteAck('tactical');
    expect(meter.drain()).toHaveLength(1);
    expect(meter.drain()).toEqual([]);
  });
});

describe('consoleLatencyMessage', () => {
  it('builds the ClientMessage::ReportConsoleLatency envelope', () => {
    const samples = [
      { action: 'fire_phaser', surface: 'PhoneConsole', input_to_send_ms: 1, send_to_ack_ms: 2 },
    ];
    expect(consoleLatencyMessage(samples)).toEqual({
      type: 'ReportConsoleLatency',
      data: { samples },
    });
  });

  /// Mirrors `core::messages::MAX_CONSOLE_LATENCY_SAMPLES`: the host truncates
  /// at the same number, so sending more would only be discarded.
  it('truncates to the wire batch limit', () => {
    const many = Array.from({ length: MAX_BATCH + 20 }, (_, i) => ({ action: `a${i}` }));
    expect(consoleLatencyMessage(many).data.samples).toHaveLength(MAX_BATCH);
  });
});

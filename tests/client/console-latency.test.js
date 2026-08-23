// Issue #1169 — client-side console input-to-feedback measurement.
//
// The meter lives in gui/console-latency.js precisely so it can be driven here
// without a browser, a phone or a WASM bundle: it is a small state machine over
// an injected clock. These tests pin the properties the measurement's honesty
// rests on — one clock, bounded everything, nothing invented, and (since the
// #1169 review) an ack that only a HOST-caused refresh can produce.

import { describe, it, expect } from 'vitest';
import {
  ConsoleLatencyMeter,
  ackEligible,
  consoleLatencyMessage,
  isConsoleLatencyEnabled,
  nowMs,
  MAX_BATCH,
  PUSH_CAUSE,
} from '../../gui/console-latency.js';

/** A meter over a clock the test advances by hand. */
function meterAt(opts = {}) {
  const clock = { t: 1000 };
  const meter = new ConsoleLatencyMeter({ now: () => clock.t, ...opts });
  meter.setEnabled(true);
  return { meter, clock };
}

/** Dispatch one action and acknowledge it from the host, `ms` later. */
function roundTrip(meter, clock, action, target, ms) {
  meter.noteDispatch(action, target, clock.t);
  clock.t += ms;
  meter.noteAck(target, PUSH_CAUSE.SERVER_MESSAGE);
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

// ── Ack eligibility (issue #1169 review, finding B1) ────────────────────────
//
// A console's payload is re-pushed for several client-local reasons. Treating
// any of them as an acknowledgement records a plausible duration for a command
// the host may never have received — a 40 ms "ack" across a stalled channel.

describe('ackEligible', () => {
  it('only a host-caused push acknowledges an action', () => {
    expect(ackEligible(PUSH_CAUSE.SERVER_MESSAGE)).toBe(true);
  });

  it('refuses every client-local push cause', () => {
    expect(ackEligible(PUSH_CAUSE.RENDER)).toBe(false);
    expect(ackEligible(PUSH_CAUSE.IFRAME_LOAD)).toBe(false);
    expect(ackEligible(PUSH_CAUSE.TUTORIAL)).toBe(false);
  });

  it('fails safe on an unnamed or unknown cause', () => {
    expect(ackEligible(undefined)).toBe(false);
    expect(ackEligible(null)).toBe(false);
    expect(ackEligible('something-new')).toBe(false);
  });
});

describe('ConsoleLatencyMeter — only the host can acknowledge', () => {
  it('a render-driven re-push records nothing', () => {
    const { meter, clock } = meterAt();
    meter.noteDispatch('fire_phaser', 'tactical', clock.t);
    clock.t += 40;
    // Rotation, resize, tab switch — the data channel may be stalled and the
    // host may never have seen the command.
    meter.noteAck('tactical', PUSH_CAUSE.RENDER);
    expect(meter.drain().samples).toEqual([]);
    expect(meter.pendingCount()).toBe(1);
  });

  it('an iframe-load re-push records nothing', () => {
    const { meter, clock } = meterAt();
    meter.noteDispatch('fire_phaser', 'tactical', clock.t);
    clock.t += 40;
    meter.noteAck('tactical', PUSH_CAUSE.IFRAME_LOAD);
    expect(meter.drain().samples).toEqual([]);
  });

  it('a client-local tutorial push records nothing', () => {
    const { meter, clock } = meterAt();
    meter.noteDispatch('fire_phaser', 'tactical', clock.t);
    clock.t += 40;
    meter.noteAck('tactical', PUSH_CAUSE.TUTORIAL);
    expect(meter.drain().samples).toEqual([]);
  });

  it('the action survives a client-local push and is measured on the real one', () => {
    const { meter, clock } = meterAt();
    meter.noteDispatch('fire_phaser', 'tactical', clock.t);
    clock.t += 40;
    meter.noteAck('tactical', PUSH_CAUSE.RENDER);
    clock.t += 20;
    meter.noteAck('tactical', PUSH_CAUSE.SERVER_MESSAGE);
    // 60 ms of real waiting, not the 40 ms the render would have claimed.
    expect(meter.drain().samples[0].send_to_ack_ms).toBe(60);
  });
});

describe('ConsoleLatencyMeter — the two segments', () => {
  it('splits client-local work from the round trip', () => {
    const { meter, clock } = meterAt();
    // The console stamped its input event 3 ms before the shell dispatched.
    meter.noteDispatch('fire_phaser', 'tactical', clock.t - 3);
    clock.t += 47;
    meter.noteAck('tactical', PUSH_CAUSE.SERVER_MESSAGE);

    const { samples } = meter.drain();
    // No `surface` field: the host assigns that, so a client cannot claim a
    // series it does not own.
    expect(samples).toEqual([
      { action: 'fire_phaser', input_to_send_ms: 3, send_to_ack_ms: 47 },
    ]);
  });

  it('reports zero client-local time rather than guessing when nothing stamped', () => {
    const { meter, clock } = meterAt();
    // A dispatch from something that is not a console control carries no
    // `__input_ms`.
    meter.noteDispatch('select_scenario', 'tactical', undefined);
    clock.t += 20;
    meter.noteAck('tactical', PUSH_CAUSE.SERVER_MESSAGE);

    const [sample] = meter.drain().samples;
    expect(sample.input_to_send_ms).toBe(0);
    expect(sample.send_to_ack_ms).toBe(20);
  });

  it('resolves every action pending on the surface that refreshed', () => {
    const { meter, clock } = meterAt();
    meter.noteDispatch('fire_phaser', 'tactical', clock.t);
    clock.t += 5;
    meter.noteDispatch('set_target', 'tactical', clock.t);
    clock.t += 30;
    meter.noteAck('tactical', PUSH_CAUSE.SERVER_MESSAGE);

    const { samples } = meter.drain();
    expect(samples.map((s) => s.action)).toEqual(['fire_phaser', 'set_target']);
    expect(samples.map((s) => s.send_to_ack_ms)).toEqual([35, 30]);
  });

  it('a refresh of a DIFFERENT console acknowledges nothing', () => {
    const { meter, clock } = meterAt();
    meter.noteDispatch('fire_phaser', 'tactical', clock.t);
    clock.t += 10;
    meter.noteAck('repair', PUSH_CAUSE.SERVER_MESSAGE);

    expect(meter.drain().samples).toEqual([]);
    expect(meter.pendingCount()).toBe(1);
  });
});

describe('ConsoleLatencyMeter — the host owns the switch', () => {
  it('measures nothing until enabled', () => {
    const clock = { t: 0 };
    const meter = new ConsoleLatencyMeter({ now: () => clock.t });
    expect(meter.enabled).toBe(false);
    meter.noteDispatch('fire_phaser', 'tactical', 0);
    clock.t += 50;
    meter.noteAck('tactical', PUSH_CAUSE.SERVER_MESSAGE);
    expect(meter.hasPayload()).toBe(false);
    expect(meter.pendingCount()).toBe(0);
  });

  // Turning off mid-flight discards everything: an ack landing after a re-enable
  // would carry a `send_to_ack` spanning the gap, and a fabricated outlier is
  // worse than a missing sample. The host's tracker clears on the same edge.
  it('discards work in flight and undelivered counts when switched off', () => {
    const { meter, clock } = meterAt();
    meter.noteDispatch('fire_phaser', 'tactical', clock.t);
    roundTrip(meter, clock, 'set_target', 'tactical', 10);
    meter.setEnabled(false);
    meter.setEnabled(true);
    clock.t += 5000;
    meter.noteAck('tactical', PUSH_CAUSE.SERVER_MESSAGE);
    expect(meter.hasPayload()).toBe(false);
  });
});

// ── Outage counting (issue #1169 review, finding C1) ────────────────────────
//
// An unanswered action yields no duration, so silently dropping it makes a dead
// link and a quiet link produce the same (empty) distribution.

describe('ConsoleLatencyMeter — outages are counted, not censored', () => {
  it('counts an action the surface never answers', () => {
    const { meter, clock } = meterAt({ expiryMs: 1000 });
    meter.noteDispatch('fire_phaser', 'tactical', clock.t);
    clock.t += 1500; // no host-caused refresh arrived in time
    meter.noteAck('tactical', PUSH_CAUSE.SERVER_MESSAGE);

    const { samples, expired } = meter.drain();
    expect(samples).toEqual([]);
    expect(expired).toEqual([{ action: 'fire_phaser', count: 1 }]);
    expect(meter.pendingCount()).toBe(0);
  });

  it('aggregates repeated outages per action', () => {
    const { meter, clock } = meterAt({ expiryMs: 1000 });
    for (const action of ['fire_phaser', 'fire_phaser', 'set_target']) {
      meter.noteDispatch(action, 'tactical', clock.t);
    }
    clock.t += 1500;
    meter.noteAck('tactical', PUSH_CAUSE.RENDER); // expiry runs on any cause

    const { expired } = meter.drain();
    expect(expired).toEqual([
      { action: 'fire_phaser', count: 2 },
      { action: 'set_target', count: 1 },
    ]);
  });

  it('an action dropped by the per-target cap is counted too', () => {
    const { meter, clock } = meterAt({ maxPending: 2 });
    for (const action of ['a', 'b', 'c']) {
      meter.noteDispatch(action, 'tactical', clock.t);
      clock.t += 1;
    }
    expect(meter.pendingCount()).toBe(2);
    meter.noteAck('tactical', PUSH_CAUSE.SERVER_MESSAGE);
    const { samples, expired } = meter.drain();
    expect(samples.map((s) => s.action)).toEqual(['b', 'c']);
    expect(expired).toEqual([{ action: 'a', count: 1 }]);
  });

  it('a healthy round trip reports no outage', () => {
    const { meter, clock } = meterAt();
    roundTrip(meter, clock, 'fire_phaser', 'tactical', 40);
    expect(meter.drain().expired).toEqual([]);
  });
});

describe('ConsoleLatencyMeter — everything is bounded', () => {
  it('caps the outgoing batch at the wire limit, keeping the most recent', () => {
    const { meter, clock } = meterAt({ maxPending: 200, maxBatch: 3 });
    for (let i = 0; i < 10; i += 1) {
      meter.noteDispatch(`a${i}`, 'tactical', clock.t);
    }
    meter.noteAck('tactical', PUSH_CAUSE.SERVER_MESSAGE);
    const { samples } = meter.drain();
    expect(samples).toHaveLength(3);
    expect(samples.map((s) => s.action)).toEqual(['a7', 'a8', 'a9']);
  });

  it('drain empties the meter', () => {
    const { meter, clock } = meterAt();
    roundTrip(meter, clock, 'fire_phaser', 'tactical', 5);
    expect(meter.drain().samples).toHaveLength(1);
    expect(meter.drain().samples).toEqual([]);
    expect(meter.hasPayload()).toBe(false);
  });

  it('hasPayload is true for an outage with no successful sample', () => {
    const { meter, clock } = meterAt({ expiryMs: 10 });
    meter.noteDispatch('fire_phaser', 'tactical', clock.t);
    clock.t += 100;
    meter.noteAck('tactical', PUSH_CAUSE.RENDER);
    expect(meter.hasPayload()).toBe(true);
  });
});

describe('consoleLatencyMessage', () => {
  it('builds the ClientMessage::ReportConsoleLatency envelope', () => {
    const samples = [{ action: 'fire_phaser', input_to_send_ms: 1, send_to_ack_ms: 2 }];
    const expired = [{ action: 'set_impulse', count: 4 }];
    expect(consoleLatencyMessage(samples, expired)).toEqual({
      type: 'ReportConsoleLatency',
      data: { samples, expired },
    });
  });

  it('defaults the outage list, so a caller may omit it', () => {
    expect(consoleLatencyMessage([]).data.expired).toEqual([]);
  });

  // Mirrors `core::messages::MAX_CONSOLE_LATENCY_SAMPLES`: the host folds at
  // most this many from each list, so sending more would only be discarded.
  it('truncates both lists to the wire batch limit', () => {
    const many = Array.from({ length: MAX_BATCH + 20 }, (_, i) => ({ action: `a${i}` }));
    const msg = consoleLatencyMessage(many, many);
    expect(msg.data.samples).toHaveLength(MAX_BATCH);
    expect(msg.data.expired).toHaveLength(MAX_BATCH);
  });

  it('carries no surface for a client to forge', () => {
    const encoded = JSON.stringify(
      consoleLatencyMessage([{ action: 'x', input_to_send_ms: 0, send_to_ack_ms: 1 }]),
    );
    expect(encoded).not.toContain('surface');
    expect(encoded).not.toContain('SimHost');
  });
});

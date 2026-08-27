/**
 * gui/console-latency.js — client-side console input-to-feedback measurement
 * (issue #1169, PRD #1144).
 *
 * The polish bar PRD #1144 sets is "a frequent action acknowledges within
 * ~100 ms". Nothing measured that: `browser.frame` times the ECS schedule, not
 * a round trip, and a phone console was not measured at all. This module is the
 * client half — the half that can see a player's input event and the moment
 * their surface answers.
 *
 * ## One clock, always
 *
 * Every number this produces is a DIFFERENCE between two stamps taken on the
 * SAME device. That is not a convenience, it is the only way the numbers mean
 * anything: a phone's clock and the host's clock have no defined relationship,
 * so any host-side subtraction of a phone-side stamp would measure how far apart
 * the two clocks are set as much as how long the player waited. What crosses the
 * wire (`ClientMessage::ReportConsoleLatency`) is therefore durations, never
 * timestamps — and no surface name either: the host assigns that from the fact
 * that the report arrived over a session at all, so a client cannot file against
 * a series it does not own.
 *
 * Even within one device, `performance.now()` is relative to each DOCUMENT's own
 * `timeOrigin`, and a console iframe's origin is not its parent's — so
 * differencing a raw `performance.now()` taken in a console against one taken in
 * the shell would measure how much later the iframe was created. {@link nowMs}
 * adds `timeOrigin` to put every document on the device on one axis.
 *
 * ## The two segments, and what they honestly cover
 *
 * - `input_to_send` — from the console control's handler raising the action
 *   (stamped in `gui/console-core.js`'s `sendAction`) to the shell handing the
 *   envelope to its transport. Client-local work: postMessage hop, the
 *   action-map lookup, envelope construction.
 * - `send_to_ack` — from that hand-off until the issuing console's surface is
 *   next handed fresh server-derived state. The whole round trip: transport out,
 *   the host's wait for its next fixed tick, admission, the tick, the broadcast,
 *   transport back, and the client's own fold.
 *
 * ### `send_to_ack` is a perceived-feedback proxy, not per-command service time
 *
 * The client cannot see which broadcast its command caused. A `SimState` alone
 * dirties several consoles at 10 Hz, so what this actually measures is *the wait
 * until the surface next showed something new* — a figure bounded below by the
 * host's broadcast cadence rather than by how long the command took to process.
 *
 * That is the right number for the ~100 ms bar, which is a claim about what a
 * player perceives. It is the WRONG number for detecting a per-command
 * processing regression: that lives in the host-side `admit_to_broadcast`
 * segment and nowhere else. (Issue #1169 review, finding C1.)
 *
 * It also stops at fresh state, not at painted pixels — no page can observe its
 * own compositor. The name is `ack`, not `paint`.
 *
 * ## Not every refresh is an acknowledgement
 *
 * A console's payload is re-pushed for several reasons that have nothing to do
 * with the host answering: a local re-render on rotation or resize, a freshly
 * (re)loaded iframe getting the current snapshot, client-local tutorial
 * progress. Treating any of those as an ack records a plausible-looking duration
 * for a command the host may never even have received — a 40 ms "ack" across a
 * stalled data channel (issue #1169 review, finding B1).
 *
 * So an ack must name its cause, and only {@link PUSH_CAUSE.SERVER_MESSAGE}
 * qualifies. {@link ackEligible} is the one place that decision is made, and it
 * lives here — not in a shell's inline script — precisely so it is testable.
 *
 * ## There is no browser-host series
 *
 * The host page (`server.html`) mounts no console surface that receives state:
 * the inbound half of its console relay went with issue #818. With no feedback
 * path there is nothing to acknowledge, so the host page runs no meter at all.
 * The first implementation acked off the `hud` host channel, which is
 * change-gated on six coarse fields — it therefore both missed real acks and
 * invented false ones from unrelated heading ticks, and an absent metric beats a
 * fabricated one (issue #1169 review, finding B2).
 *
 * Pure and DOM-free so it runs under vitest in Node; the shell injects its own
 * transport and clock.
 */

/**
 * The most records of each kind one `ReportConsoleLatency` batch carries.
 * Mirrors `core::messages::MAX_CONSOLE_LATENCY_SAMPLES` — the host folds at most
 * this many from each list, so sending more would only be discarded.
 */
export const MAX_BATCH = 64;

import { DEBUG_SURFACE } from './debug-surfaces.generated.js';

/** Canonical generated `DebugSurface` wire name this measurement is gated on. */
export const CONSOLE_LATENCY_FLAG = DEBUG_SURFACE.ConsoleLatency;

/**
 * Why a console's payload was pushed to its surface.
 *
 * The shell knows this at each call site and nothing else does, so it is passed
 * in rather than inferred. Anything not on this list is treated as
 * non-acknowledging by {@link ackEligible}, which is the safe direction: a new
 * push site that forgets to name its cause under-reports rather than inventing
 * samples.
 */
export const PUSH_CAUSE = Object.freeze({
  /** An inbound host message carried or dirtied state for this console. */
  SERVER_MESSAGE: 'server-message',
  /** A local re-render: rotation, resize, tab switch, phase change. */
  RENDER: 'render',
  /** A (re)loaded iframe being handed the current snapshot. */
  IFRAME_LOAD: 'iframe-load',
  /** Client-local tutorial progress, which never leaves the device. */
  TUTORIAL: 'tutorial',
});

/**
 * Whether a push of this cause may acknowledge a pending action.
 *
 * Only a push the HOST caused counts. The other three are client-local: they
 * would happily "acknowledge" an action while the data channel was stalled and
 * the host had never seen it, which is the exact false reading this predicate
 * exists to refuse.
 *
 * @param {string} cause a {@link PUSH_CAUSE} value
 * @returns {boolean}
 */
export function ackEligible(cause) {
  return cause === PUSH_CAUSE.SERVER_MESSAGE;
}

/** How long a shell waits between reporting batches, in milliseconds. */
export const FLUSH_INTERVAL_MS = 2000;

/**
 * Whether a shell should send its measured batch now.
 *
 * Extracted here, and unit-tested, for the same reason {@link ackEligible} is:
 * the flush policy is a DECISION — follow the host's flag, batch rather than
 * stream, and never lose the last batch to a page teardown — and it lived in a
 * shell's inline script where no test could reach it.
 *
 * Batching matters more than it looks: a tapping player would otherwise turn one
 * diagnostic into a stream of tiny frames on the very data channel whose latency
 * is being measured, so the measurement would change what it measures. `force`
 * is the teardown path (`pagehide` / `visibilitychange`), where there is no next
 * chance and the interval must not apply.
 *
 * @param {{
 *   enabled: boolean,      // the host's flag, as this client last saw it
 *   hasPayload: boolean,   // anything measured or counted since the last flush
 *   now: number,           // current clock reading
 *   lastFlushedAt: number, // clock reading at the previous flush
 *   force?: boolean,       // page teardown: send whatever there is
 *   intervalMs?: number,
 * }} state
 * @returns {boolean}
 */
export function shouldFlush({
  enabled,
  hasPayload,
  now,
  lastFlushedAt,
  force = false,
  intervalMs = FLUSH_INTERVAL_MS,
}) {
  if (!enabled || !hasPayload) return false;
  return force || now - lastFlushedAt >= intervalMs;
}

/**
 * Epoch-relative high-resolution time in milliseconds.
 *
 * `performance.timeOrigin + performance.now()` rather than either alone: the
 * first is a coarse epoch anchor, the second a fine monotonic offset from a
 * PER-DOCUMENT origin. Added, they put a console iframe and its shell — two
 * documents on one device — on a single axis, which is what makes
 * "input stamped in the console, send stamped in the shell" a real duration.
 *
 * Falls back to `Date.now()` where `performance` is absent (old embeddings, some
 * test environments): coarser, still one device, still monotonic enough for a
 * measurement whose bar is 100 ms.
 *
 * @returns {number} milliseconds
 */
export function nowMs() {
  if (typeof performance !== 'undefined' && typeof performance.now === 'function') {
    const origin = Number.isFinite(performance.timeOrigin) ? performance.timeOrigin : 0;
    return origin + performance.now();
  }
  return Date.now();
}

/**
 * Whether the host has console-latency measurement switched on.
 *
 * Read from the `ServerMessage::DebugState` fold `gui/sim-state.js` keeps
 * (`{ flags: { ConsoleLatency: bool }, ... }`). A flag the host has not reported
 * reads OFF — the client never guesses itself into measuring, exactly as the
 * settings panel never guesses a toggle's state.
 *
 * @param {{flags?: Object<string, boolean>}|null|undefined} debug
 * @returns {boolean}
 */
export function isConsoleLatencyEnabled(debug) {
  return !!(debug && debug.flags && debug.flags[CONSOLE_LATENCY_FLAG]);
}

/**
 * The `ClientMessage::ReportConsoleLatency` envelope for one batch.
 *
 * @param {Array<object>} samples  measured round trips
 * @param {Array<object>} [expired] per-action counts of actions never answered
 * @returns {{type: string, data: {samples: Array<object>, expired: Array<object>}}}
 */
export function consoleLatencyMessage(samples, expired = []) {
  return {
    type: 'ReportConsoleLatency',
    data: {
      samples: samples.slice(0, MAX_BATCH),
      expired: expired.slice(0, MAX_BATCH),
    },
  };
}

/**
 * Measures console actions on this device and batches the results.
 *
 * Lifecycle per action:
 *
 *   noteDispatch(action, target, inputMs)      ← the shell is about to transmit
 *        ... server round trip ...
 *   noteAck(target, PUSH_CAUSE.SERVER_MESSAGE) ← the host answered that surface
 *        → one sample joins the outgoing batch
 *
 * An action the host never answers is dropped after `expiryMs` and COUNTED. It
 * has to be counted: an unanswered action yields no duration, so an outage would
 * otherwise be invisible — a link that stopped delivering and a link nobody is
 * using produce the same (empty) distribution. The count travels with the batch
 * and lands beside the distributions it qualifies.
 *
 * Everything is bounded, because everything here is driven by a player who may
 * tap faster than the host answers: at most `maxPending` unanswered actions per
 * target, at most `maxBatch` finished samples waiting to be sent. Dropping the
 * OLDEST in each case is deliberate — a surface that stopped measuring because
 * it fell behind would go quiet exactly when it got interesting.
 */
export class ConsoleLatencyMeter {
  /**
   * @param {{
   *   maxPending?: number,  // unanswered actions retained per target
   *   maxBatch?: number,    // finished samples retained before a flush
   *   expiryMs?: number,    // how long an unanswered action stays pending
   *   now?: () => number,   // clock, injectable for tests
   * }} [opts]
   */
  constructor({ maxPending = 8, maxBatch = MAX_BATCH, expiryMs = 5000, now = nowMs } = {}) {
    this.maxPending = maxPending;
    this.maxBatch = maxBatch;
    this.expiryMs = expiryMs;
    this._now = now;
    this._enabled = false;
    /** @type {Map<string, Array<{action: string, sendMs: number, inputToSend: number}>>} */
    this._pending = new Map();
    /** @type {Array<object>} */
    this._samples = [];
    /** @type {Map<string, number>} action → unanswered count since the last drain */
    this._expired = new Map();
  }

  get enabled() {
    return this._enabled;
  }

  /**
   * Switch measurement on or off, following the host's reported flag.
   *
   * A CHANGE either way discards everything in flight and every undelivered
   * count. A half-measured action whose ack lands after a re-enable would carry
   * a `send_to_ack` spanning the gap, and a made-up outlier is worse than a
   * missing sample. The host's tracker clears itself on the same edge, so the
   * two halves start each measurement session together.
   *
   * @param {boolean} on
   * @returns {boolean} the new state
   */
  setEnabled(on) {
    const next = !!on;
    if (next !== this._enabled) {
      this._pending.clear();
      this._samples.length = 0;
      this._expired.clear();
    }
    this._enabled = next;
    return this._enabled;
  }

  /**
   * Record that an action is being handed to the transport now.
   *
   * @param {string} action    the console action name (`gui/action-map.js` key)
   * @param {string} target    the console whose next server-caused refresh acks it
   * @param {number} [inputMs] the input-event stamp from `console-core.sendAction`
   */
  noteDispatch(action, target, inputMs) {
    if (!this._enabled || typeof action !== 'string' || !action) return;
    const sendMs = this._now();
    // An action raised by something that is not a console control (a shell's own
    // scenario picker, a synthetic dispatch) carries no input stamp. Report zero
    // client-local time rather than guessing one: the round trip is still worth
    // having, and a fabricated first segment would inflate the end-to-end figure
    // the ~100 ms bar is read against.
    const inputToSend =
      Number.isFinite(inputMs) && inputMs > 0 ? Math.max(0, sendMs - inputMs) : 0;
    const key = String(target || '');
    const queue = this._pending.get(key) || [];
    queue.push({ action, sendMs, inputToSend });
    // Over the per-target cap the oldest is dropped — and counted, because it is
    // an action this surface never answered.
    while (queue.length > this.maxPending) this._countExpired(queue.shift());
    this._pending.set(key, queue);
  }

  /**
   * Record that `target`'s surface has been handed a payload, and resolve the
   * actions waiting on it **if the push was caused by the host**.
   *
   * A render-driven, iframe-load or tutorial push is not an acknowledgement of
   * anything: see {@link ackEligible}. Expiry still runs for any cause, so a
   * long-unanswered action is retired on the next push whatever caused it.
   *
   * @param {string} target
   * @param {string} cause a {@link PUSH_CAUSE} value
   */
  noteAck(target, cause) {
    if (!this._enabled) return;
    const ackMs = this._now();
    this._expire(ackMs);
    if (!ackEligible(cause)) return;
    const key = String(target || '');
    const queue = this._pending.get(key);
    if (!queue || queue.length === 0) return;
    for (const item of queue) {
      this._push({
        action: item.action,
        input_to_send_ms: item.inputToSend,
        send_to_ack_ms: Math.max(0, ackMs - item.sendMs),
      });
    }
    this._pending.delete(key);
  }

  /**
   * Take the finished samples and outage counts, leaving the meter empty.
   *
   * @returns {{samples: Array<object>, expired: Array<{action: string, count: number}>}}
   */
  drain() {
    const samples = this._samples;
    const expired = Array.from(this._expired, ([action, count]) => ({ action, count }));
    this._samples = [];
    this._expired.clear();
    return { samples, expired };
  }

  /** Whether a flush would carry anything. */
  hasPayload() {
    return this._samples.length > 0 || this._expired.size > 0;
  }

  /** How many actions are still waiting for an acknowledgement. */
  pendingCount() {
    let total = 0;
    for (const queue of this._pending.values()) total += queue.length;
    return total;
  }

  /**
   * Retire actions the host never answered, counting each.
   *
   * An action whose surface never refreshed produced no observable feedback at
   * all. Turning that into a multi-second duration would poison the very tail
   * the surface exists to show, and dropping it silently would hide an outage —
   * so it leaves the distributions and joins the count.
   *
   * @param {number} nowMsValue
   */
  _expire(nowMsValue) {
    for (const [key, queue] of this._pending) {
      const kept = [];
      for (const item of queue) {
        if (nowMsValue - item.sendMs <= this.expiryMs) kept.push(item);
        else this._countExpired(item);
      }
      if (kept.length === 0) this._pending.delete(key);
      else if (kept.length !== queue.length) this._pending.set(key, kept);
    }
  }

  /** @param {{action: string}|undefined} item */
  _countExpired(item) {
    if (!item) return;
    this._expired.set(item.action, (this._expired.get(item.action) || 0) + 1);
  }

  /** @param {object} sample */
  _push(sample) {
    this._samples.push(sample);
    while (this._samples.length > this.maxBatch) this._samples.shift();
  }
}

// Expose for the classic-script bootstrap in client.html, which wires the meter
// into its own dispatch and console-refresh paths.
if (typeof window !== 'undefined') {
  window.ConsoleLatencyMeter = ConsoleLatencyMeter;
  window.consoleLatencyMessage = consoleLatencyMessage;
  window.isConsoleLatencyEnabled = isConsoleLatencyEnabled;
  window.consoleLatencyNowMs = nowMs;
  window.CONSOLE_PUSH_CAUSE = PUSH_CAUSE;
  window.consoleLatencyShouldFlush = shouldFlush;
}

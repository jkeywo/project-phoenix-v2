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
 * timestamps.
 *
 * Even within one device, `performance.now()` is relative to each DOCUMENT's own
 * `timeOrigin`, and a console iframe's origin is not its parent's — so
 * differencing a raw `performance.now()` taken in a console against one taken in
 * the shell would measure how much later the iframe was created. {@link nowMs}
 * adds `timeOrigin` to put every document on the device on one axis.
 *
 * ## The two segments, and what they honestly cover
 *
 * - `input_to_send` — the console control's handler raising the action (stamped
 *   in `gui/console-core.js`'s `sendAction`) → the shell handing the envelope to
 *   its transport. Client-local work: postMessage hop, the action-map lookup,
 *   envelope construction.
 * - `send_to_ack` — that hand-off → the moment the acknowledging surface is next
 *   handed fresh server-derived state. The whole round trip: transport out, the
 *   host's wait for its next fixed tick, admission, the tick, the broadcast,
 *   transport back, and the client's own fold.
 *
 * **It stops at fresh state, not at pixels.** No code in a page can observe its
 * own compositor, so nothing here claims to. The name is `ack`, not `paint`.
 *
 * ## What counts as the acknowledging surface differs per path
 *
 * - **Phone console** (`client.html`): the ISSUING console's own next state
 *   push. `pushConsoleStateFor(name)` is the one place a console's payload is
 *   rebuilt and handed to its iframe, so it is the ack.
 * - **Browser host** (`server.html`): the host page has no console iframes — it
 *   is the viewscreen. Its equivalent surface is the `hud` host channel, the one
 *   push that hands the page fresh server-derived ship state.
 *
 * Both are "the surface the player is looking at got a new answer". Neither is
 * a causal proof that THIS action caused THAT refresh — a broadcast the action
 * did not cause can arrive first — so the number is honestly "how long until the
 * issuing surface next showed something new", which is what a player perceives
 * and is what the ~100 ms bar is about.
 *
 * Pure and DOM-free so it runs under vitest in Node; the shells inject their own
 * transport and clock.
 */

/**
 * The most samples one `ReportConsoleLatency` batch carries.
 * Mirrors `core::messages::MAX_CONSOLE_LATENCY_SAMPLES` — the host truncates at
 * the same number, so sending more would only be discarded.
 */
export const MAX_BATCH = 64;

/** `DebugFlag` variant name this measurement is gated on. */
export const CONSOLE_LATENCY_FLAG = 'ConsoleLatency';

/** The `LatencySurface` variant names, as the Rust enum spells them. */
export const SURFACE_BROWSER_HOST = 'BrowserHost';
export const SURFACE_PHONE_CONSOLE = 'PhoneConsole';

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
 * The `ClientMessage::ReportConsoleLatency` envelope for a batch of samples.
 *
 * @param {Array<object>} samples
 * @returns {{type: string, data: {samples: Array<object>}}}
 */
export function consoleLatencyMessage(samples) {
  return { type: 'ReportConsoleLatency', data: { samples: samples.slice(0, MAX_BATCH) } };
}

/**
 * Measures console actions on one client surface and batches the results.
 *
 * Lifecycle per action:
 *
 *   noteDispatch(action, target, inputMs)   ← the shell is about to transmit
 *        ... server round trip ...
 *   noteAck(target)                         ← that surface got fresh state
 *        → one sample joins the outgoing batch
 *
 * Everything is bounded, because everything here is driven by a player who may
 * tap faster than the host answers: at most `maxPending` unanswered actions per
 * target, at most `maxBatch` finished samples waiting to be sent, and an
 * unanswered action is discarded after `expiryMs` rather than held forever.
 * Dropping the OLDEST in each case is deliberate — a surface that stopped
 * measuring because it fell behind would go quiet exactly when it got
 * interesting.
 */
export class ConsoleLatencyMeter {
  /**
   * @param {{
   *   surface: string,      // a `LatencySurface` variant name
   *   maxPending?: number,  // unanswered actions retained per target
   *   maxBatch?: number,    // finished samples retained before a flush
   *   expiryMs?: number,    // how long an unanswered action stays pending
   *   now?: () => number,   // clock, injectable for tests
   * }} opts
   */
  constructor({ surface, maxPending = 8, maxBatch = MAX_BATCH, expiryMs = 5000, now = nowMs } = {}) {
    this.surface = surface;
    this.maxPending = maxPending;
    this.maxBatch = maxBatch;
    this.expiryMs = expiryMs;
    this._now = now;
    this._enabled = false;
    /** @type {Map<string, Array<{action: string, sendMs: number, inputToSend: number}>>} */
    this._pending = new Map();
    /** @type {Array<object>} */
    this._samples = [];
  }

  get enabled() {
    return this._enabled;
  }

  /**
   * Switch measurement on or off, following the host's reported flag.
   *
   * Switching OFF discards everything in flight. A half-measured action whose
   * ack lands after a re-enable would carry a `send_to_ack` spanning the gap,
   * and a made-up outlier is worse than a missing sample.
   *
   * @param {boolean} on
   * @returns {boolean} the new state
   */
  setEnabled(on) {
    const next = !!on;
    if (next !== this._enabled) {
      this._pending.clear();
      this._samples.length = 0;
    }
    this._enabled = next;
    return this._enabled;
  }

  /**
   * Record that an action is being handed to the transport now.
   *
   * @param {string} action    the console action name (`gui/action-map.js` key)
   * @param {string} target    the surface whose next refresh acknowledges it
   * @param {number} [inputMs] the input-event stamp from `console-core.sendAction`
   */
  noteDispatch(action, target, inputMs) {
    if (!this._enabled || typeof action !== 'string' || !action) return;
    const sendMs = this._now();
    // An action raised by something that is not a console control (the host
    // page's own scenario picker, a synthetic dispatch) carries no input stamp.
    // Report zero client-local time rather than guessing one: the round trip is
    // still worth having, and a fabricated first segment would inflate the
    // end-to-end figure the ~100 ms bar is read against.
    const inputToSend =
      Number.isFinite(inputMs) && inputMs > 0 ? Math.max(0, sendMs - inputMs) : 0;
    const key = String(target || '');
    const queue = this._pending.get(key) || [];
    queue.push({ action, sendMs, inputToSend });
    while (queue.length > this.maxPending) queue.shift();
    this._pending.set(key, queue);
  }

  /**
   * Record that `target`'s surface has just been handed fresh server state.
   * Every action still waiting on that surface resolves into a sample.
   *
   * @param {string} target
   */
  noteAck(target) {
    if (!this._enabled) return;
    const ackMs = this._now();
    this._expire(ackMs);
    const key = String(target || '');
    const queue = this._pending.get(key);
    if (!queue || queue.length === 0) return;
    for (const item of queue) {
      this._push({
        action: item.action,
        surface: this.surface,
        input_to_send_ms: item.inputToSend,
        send_to_ack_ms: Math.max(0, ackMs - item.sendMs),
      });
    }
    this._pending.delete(key);
  }

  /**
   * Take the finished samples, leaving the meter empty.
   * @returns {Array<object>} oldest first; empty when there is nothing to send
   */
  drain() {
    const out = this._samples;
    this._samples = [];
    return out;
  }

  /** How many actions are still waiting for an acknowledgement. */
  pendingCount() {
    let total = 0;
    for (const queue of this._pending.values()) total += queue.length;
    return total;
  }

  /**
   * Drop actions that were never acknowledged.
   *
   * An action whose surface never refreshed produced no observable feedback at
   * all. That is a real finding, but it is not a LATENCY, and turning it into a
   * multi-second sample would poison the very tail the surface exists to show.
   *
   * @param {number} nowMsValue
   */
  _expire(nowMsValue) {
    for (const [key, queue] of this._pending) {
      const kept = queue.filter((item) => nowMsValue - item.sendMs <= this.expiryMs);
      if (kept.length === 0) this._pending.delete(key);
      else if (kept.length !== queue.length) this._pending.set(key, kept);
    }
  }

  /** @param {object} sample */
  _push(sample) {
    this._samples.push(sample);
    while (this._samples.length > this.maxBatch) this._samples.shift();
  }
}

// Expose for the classic-script bootstraps in client.html / server.html, which
// wire the meter into their own dispatch and refresh paths.
if (typeof window !== 'undefined') {
  window.ConsoleLatencyMeter = ConsoleLatencyMeter;
  window.consoleLatencyMessage = consoleLatencyMessage;
  window.isConsoleLatencyEnabled = isConsoleLatencyEnabled;
  window.consoleLatencyNowMs = nowMs;
}

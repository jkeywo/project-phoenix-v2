/**
 * Correlated semantic-action feedback (issue #1276).
 *
 * This module owns presentation state only.  It never mutates simulation
 * state: a pending action remains presentation-only until the ordinary
 * authoritative console blackboard publishes the resulting gameplay state.
 *
 * Timestamps are epoch-relative stamps taken on this one device (the same
 * contract as console-latency's `nowMs`).  They are carried back to the
 * parent only so the original press can feed local latency measurement; no
 * host/client clock comparison is ever made.
 */

export const ACTION_FEEDBACK_STATE = Object.freeze({
  PRESSED: 'Pressed',
  PENDING: 'Pending',
  APPLIED: 'Applied',
  REFUSED: 'Refused',
  TIMED_OUT: 'TimedOut',
});

export const MAX_ACTION_CORRELATION_BYTES = 64;
export const DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS = 5000;
export const DEFAULT_ACTION_FEEDBACK_CAPACITY = 32;

const FINAL_STATES = new Set([
  ACTION_FEEDBACK_STATE.APPLIED,
  ACTION_FEEDBACK_STATE.REFUSED,
  ACTION_FEEDBACK_STATE.TIMED_OUT,
]);

const PRESENTATION = Object.freeze({
  [ACTION_FEEDBACK_STATE.PRESSED]: Object.freeze({
    statusId: 'action_feedback.pressed',
    cue: 'action-pressed',
    vibrationIntent: null,
  }),
  [ACTION_FEEDBACK_STATE.PENDING]: Object.freeze({
    statusId: 'action_feedback.pending',
    cue: 'action-pending',
    vibrationIntent: null,
  }),
  [ACTION_FEEDBACK_STATE.APPLIED]: Object.freeze({
    statusId: 'action_feedback.applied',
    cue: 'action-applied',
    vibrationIntent: 'confirm',
  }),
  [ACTION_FEEDBACK_STATE.REFUSED]: Object.freeze({
    statusId: 'action_feedback.refused',
    cue: 'action-refused',
    vibrationIntent: 'refuse',
  }),
  [ACTION_FEEDBACK_STATE.TIMED_OUT]: Object.freeze({
    statusId: 'action_feedback.timed_out',
    cue: 'action-timed-out',
    vibrationIntent: 'timeout',
  }),
});

export const ACTION_FEEDBACK_PREFERENCE_DEFAULTS = Object.freeze({
  vibration: true,
  semanticCues: true,
});

export function normalizeActionFeedbackPreferences(value) {
  const source = value && typeof value === 'object' ? value : {};
  return Object.freeze({
    vibration: typeof source.vibration === 'boolean'
      ? source.vibration : ACTION_FEEDBACK_PREFERENCE_DEFAULTS.vibration,
    semanticCues: typeof source.semanticCues === 'boolean'
      ? source.semanticCues : ACTION_FEEDBACK_PREFERENCE_DEFAULTS.semanticCues,
  });
}

let fallbackSequence = 0;

/** True for the exact bounded opaque string shape the Rust wire accepts. */
export function isValidActionCorrelation(value) {
  if (typeof value !== 'string' || value.length === 0) return false;
  if (new TextEncoder().encode(value).length > MAX_ACTION_CORRELATION_BYTES) return false;
  // Correlations are identifiers, not display text.  Visible ASCII avoids
  // invisible aliases while leaving their contents opaque to every consumer.
  return /^[\x21-\x7e]+$/.test(value);
}

/** Mint an opaque, wire-bounded identity. */
export function createActionCorrelation(cryptoLike = globalThis.crypto) {
  let value = '';
  if (cryptoLike && typeof cryptoLike.randomUUID === 'function') {
    value = cryptoLike.randomUUID();
  } else if (cryptoLike && typeof cryptoLike.getRandomValues === 'function') {
    const bytes = cryptoLike.getRandomValues(new Uint8Array(16));
    value = Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
  } else {
    // Tests and old embeddings may lack Web Crypto.  This is correlation, not
    // authority or a secret: uniqueness on this document is the requirement.
    fallbackSequence = (fallbackSequence + 1) >>> 0;
    value = `local-${Date.now().toString(36)}-${fallbackSequence.toString(36)}`;
  }
  if (!isValidActionCorrelation(value)) {
    throw new Error('action feedback correlation generator returned an invalid identity');
  }
  return value;
}

/** Presentation metadata derived from the lifecycle state in one place. */
export function presentationForActionFeedback(state) {
  const presentation = PRESENTATION[state];
  if (!presentation) throw new TypeError(`unknown action feedback state: ${state}`);
  return presentation;
}

function transition(record, state, isCurrent) {
  return Object.freeze({
    actionId: record.actionId,
    correlation: record.correlation,
    inputMs: record.inputMs,
    state,
    isCurrent,
    lifecycleTransition: true,
    presentationRestored: false,
    ...presentationForActionFeedback(state),
  });
}

/** Restore current presentation without replaying the state's cue or haptic. */
function restoredPresentation(record) {
  const presentation = presentationForActionFeedback(record.state);
  return Object.freeze({
    actionId: record.actionId,
    correlation: record.correlation,
    inputMs: record.inputMs,
    state: record.state,
    isCurrent: true,
    lifecycleTransition: false,
    presentationRestored: true,
    statusId: presentation.statusId,
    cue: null,
    vibrationIntent: null,
  });
}

/**
 * Iframe-side Pressed -> Pending -> terminal state machine.
 *
 * Terminal records remain in the bounded map so a late or duplicate response
 * cannot resurrect them.  Overlapping presses retain separate correlations;
 * `isCurrent` tells the visible control whether a reply belongs to its newest
 * press while every exact reply can still settle latency independently.
 */
export class ActionFeedbackLifecycle {
  constructor({
    now = Date.now,
    correlation = createActionCorrelation,
    capacity = DEFAULT_ACTION_FEEDBACK_CAPACITY,
    onTransition = () => {},
  } = {}) {
    this.now = now;
    this.correlation = correlation;
    this.capacity = Math.max(1, Number.isInteger(capacity) ? capacity : DEFAULT_ACTION_FEEDBACK_CAPACITY);
    this.onTransition = onTransition;
    this.records = new Map();
    this.latestByAction = new Map();
  }

  press(actionId) {
    const id = String(actionId || '');
    if (!id) throw new TypeError('action feedback requires an action id');
    const correlation = this.correlation();
    if (!isValidActionCorrelation(correlation) || this.records.has(correlation)) {
      throw new Error('action feedback correlation must be valid and unique');
    }
    this._makeRoom();
    const inputMs = this.now();
    if (!Number.isFinite(inputMs)) throw new TypeError('action feedback clock must be finite');
    const record = { actionId: id, correlation, inputMs, state: ACTION_FEEDBACK_STATE.PRESSED };
    this.records.set(correlation, record);
    this.latestByAction.set(id, correlation);
    this._emit(record, ACTION_FEEDBACK_STATE.PRESSED);
    return Object.freeze({ correlation, inputMs });
  }

  pending(correlation) {
    const record = this.records.get(correlation);
    if (!record || record.state !== ACTION_FEEDBACK_STATE.PRESSED) return false;
    record.state = ACTION_FEEDBACK_STATE.PENDING;
    this._emit(record, ACTION_FEEDBACK_STATE.PENDING);
    return true;
  }

  /**
   * Remove a provisional record without presenting a terminal result.
   *
   * Registry handled-false uses the Pressed arm; an async local chooser uses
   * the Pending arm when the operator cancels before any work is attempted.
   */
  cancel(correlation) {
    const record = this.records.get(correlation);
    if (!record || FINAL_STATES.has(record.state)) return false;
    const isCurrent = this.latestByAction.get(record.actionId) === correlation;
    this.records.delete(correlation);
    let promoted = null;
    if (isCurrent) {
      promoted = this._promoteLatestLive(record.actionId);
    }
    // Pressed is emitted before the adapter runs because the adapter needs the
    // correlation it creates.  A handled-false adapter (or a cancelled local
    // chooser) therefore needs an explicit presentation cleanup even though it
    // never enters a terminal lifecycle state.
    this.onTransition(Object.freeze({
      actionId: record.actionId,
      correlation: record.correlation,
      inputMs: record.inputMs,
      state: null,
      isCurrent,
      lifecycleTransition: false,
      presentationRestored: false,
      statusId: null,
      cue: null,
      vibrationIntent: null,
      cancelled: true,
    }));
    // Clearing the removed occurrence must not leave presentation blank while
    // an older overlapping occurrence is still live.  Restore its visual and
    // aria-live status without replaying the cue/haptic or claiming that its
    // underlying lifecycle entered the same state twice.
    if (promoted) this.onTransition(restoredPresentation(promoted));
    return true;
  }

  settle(correlation, state) {
    if (!FINAL_STATES.has(state)) return false;
    const record = this.records.get(correlation);
    if (!record || record.state !== ACTION_FEEDBACK_STATE.PENDING) return false;
    record.state = state;
    this._emit(record, state);
    return true;
  }

  get(correlation) {
    const record = this.records.get(correlation);
    return record ? { ...record } : null;
  }

  size() {
    return this.records.size;
  }

  _emit(record, state) {
    this.onTransition(transition(
      record,
      state,
      this.latestByAction.get(record.actionId) === record.correlation,
    ));
  }

  _makeRoom() {
    while (this.records.size >= this.capacity) {
      const terminal = [...this.records.entries()].find(([, item]) => FINAL_STATES.has(item.state));
      const oldest = terminal || this.records.entries().next().value;
      if (!oldest) return;
      const [correlation, record] = oldest;
      if (!FINAL_STATES.has(record.state)) {
        record.state = ACTION_FEEDBACK_STATE.TIMED_OUT;
        this._emit(record, ACTION_FEEDBACK_STATE.TIMED_OUT);
      }
      this.records.delete(correlation);
      if (this.latestByAction.get(record.actionId) === correlation) {
        this._promoteLatestLive(record.actionId);
      }
    }
  }

  /** Make the newest remaining provisional occurrence current after removal. */
  _promoteLatestLive(actionId) {
    const records = [...this.records.values()];
    for (let index = records.length - 1; index >= 0; index -= 1) {
      const candidate = records[index];
      if (candidate.actionId === actionId && !FINAL_STATES.has(candidate.state)) {
        this.latestByAction.set(actionId, candidate.correlation);
        return candidate;
      }
    }
    this.latestByAction.delete(actionId);
    return null;
  }
}

/**
 * Parent-side bounded correlation -> originating iframe router.
 *
 * Only an exact ActionFeedback response settles an entry.  Ordinary state
 * pushes are deliberately absent from this API, so they cannot acknowledge a
 * correlated action by accident.
 */
export class ActionFeedbackRouter {
  constructor({
    timeoutMs = DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS,
    capacity = DEFAULT_ACTION_FEEDBACK_CAPACITY,
    schedule = (fn, delay) => setTimeout(fn, delay),
    cancelSchedule = (timer) => clearTimeout(timer),
    deliver = () => {},
  } = {}) {
    this.timeoutMs = timeoutMs;
    this.capacity = Math.max(1, Number.isInteger(capacity) ? capacity : DEFAULT_ACTION_FEEDBACK_CAPACITY);
    this.schedule = schedule;
    this.cancelSchedule = cancelSchedule;
    this.deliver = deliver;
    this.pending = new Map();
  }

  track({ correlation, actionId, console: consoleName, inputMs }) {
    if (!isValidActionCorrelation(correlation) || this.pending.has(correlation)) return false;
    if (typeof actionId !== 'string' || !actionId || typeof consoleName !== 'string' || !consoleName) {
      return false;
    }
    while (this.pending.size >= this.capacity) {
      const oldest = this.pending.keys().next().value;
      this._finish(oldest, ACTION_FEEDBACK_STATE.TIMED_OUT);
    }
    const record = {
      correlation,
      actionId,
      console: consoleName,
      inputMs: Number.isFinite(inputMs) ? inputMs : null,
      timer: null,
    };
    record.timer = this.schedule(
      () => this._finish(correlation, ACTION_FEEDBACK_STATE.TIMED_OUT),
      this.timeoutMs,
    );
    this.pending.set(correlation, record);
    return true;
  }

  resolve({ correlation, outcome }) {
    const state = outcome === ACTION_FEEDBACK_STATE.APPLIED
      ? ACTION_FEEDBACK_STATE.APPLIED
      : outcome === ACTION_FEEDBACK_STATE.REFUSED
        ? ACTION_FEEDBACK_STATE.REFUSED
        : null;
    return state ? this._finish(correlation, state) : false;
  }

  size() {
    return this.pending.size;
  }

  _finish(correlation, state) {
    const record = this.pending.get(correlation);
    if (!record) return false;
    this.pending.delete(correlation);
    if (record.timer != null) this.cancelSchedule(record.timer);
    this.deliver(Object.freeze({
      actionId: record.actionId,
      correlation: record.correlation,
      inputMs: record.inputMs,
      state,
      console: record.console,
    }));
    return true;
  }
}

/**
 * Emit the shared visual/live-status value and only the effects it declares.
 * Presentation-restored values deliberately declare neither cue nor haptic.
 */
export function emitActionFeedbackTransition(root, value, preferences = null) {
  if (!root || typeof root.dispatchEvent !== 'function' || typeof CustomEvent !== 'function') return;
  const enabled = normalizeActionFeedbackPreferences(preferences);
  root.dispatchEvent(new CustomEvent('phoenix-action-feedback', { detail: value }));
  if (value.cue && enabled.semanticCues) {
    root.dispatchEvent(new CustomEvent('phoenix-semantic-cue', { detail: value }));
  }
  if (value.vibrationIntent && enabled.vibration) {
    root.dispatchEvent(new CustomEvent('phoenix-vibration-intent', { detail: value }));
  }
}

if (typeof window !== 'undefined') {
  window.ActionFeedbackRouter = ActionFeedbackRouter;
  window.ACTION_FEEDBACK_STATE = ACTION_FEEDBACK_STATE;
}

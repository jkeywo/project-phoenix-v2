import { ACTION_FEEDBACK_STATE } from './action-feedback.js';

export const gmActionEntryKey = (operatorId, correlation) => JSON.stringify([operatorId, correlation]);

/** Current GM requests and their absolute result feed. Panels own action meaning and rendering. */
export class GmActionFeedback {
  constructor({ lifecycle = () => null, capacity, timeoutMs, schedule, cancelSchedule, onLocalTerminal = () => {} }) {
    this.lifecycle = lifecycle;
    this.capacity = capacity;
    this.timeoutMs = timeoutMs;
    this.schedule = schedule;
    this.cancelSchedule = cancelSchedule;
    this.onLocalTerminal = onLocalTerminal;
    this.pending = new Map();
    this.localTerminals = new Map();
    this.authoritative = [];
  }

  get size() { return this.pending.size; }
  get authoritativeCount() { return this.authoritative.length; }
  get(correlation) { return this.pending.get(correlation); }
  values() { return this.pending.values(); }

  makeRoom(settleLifecycle = false) {
    while (this.size >= this.capacity) {
      const oldest = this.pending.keys().next().value;
      if (settleLifecycle) this.lifecycle()?.settle(oldest, ACTION_FEEDBACK_STATE.TIMED_OUT);
      this.finishLocal(oldest, 'timed-out', null);
    }
  }

  track(meta) { this.pending.set(meta.correlation, meta); }

  /** Track wire requests without exposing timer metadata to callers. */
  trackRequest(request) {
    if (this.size >= this.capacity || this.get(request.correlation)) return null;
    const meta = { request, operatorId: request.operator_id, correlation: request.correlation };
    this.track(meta);
    this.lifecycle()?.pending(meta.correlation);
    return meta;
  }
  begin(request, accepted = true, refusalReason) {
    const meta = this.trackRequest(request);
    return meta ? this.submitted(meta, accepted, refusalReason) : false;
  }
  get firstRequest() { return this.values().next().value?.request ?? null; }
  *requests() { for (const meta of this.values()) yield meta.request; }
  *localRequests() {
    for (const meta of this.localTerminals.values()) yield { ...meta.request, outcome: meta.outcome, reason: meta.reason };
  }

  clearTimer(meta) {
    if (!meta || meta.timerScheduled !== true) return;
    try { this.cancelSchedule(meta.timer); } catch (_) { /* timer already completed */ }
    meta.timer = null;
    meta.timerScheduled = false;
  }

  startTimer(meta) {
    if (meta.timerScheduled || this.get(meta.correlation) !== meta) return;
    meta.timerScheduled = true;
    meta.timer = this.schedule(() => {
      // A cancelled callback cannot affect a later request reusing its correlation.
      if (this.get(meta.correlation) !== meta) return;
      this.lifecycle()?.settle(meta.correlation, ACTION_FEEDBACK_STATE.TIMED_OUT);
      this.finishLocal(meta.correlation, 'timed-out', null);
    }, this.timeoutMs);
  }

  submitted(meta, accepted, refusalReason) {
    if (this.get(meta.correlation) !== meta) return accepted;
    this.lifecycle()?.pending(meta.correlation);
    if (accepted) this.startTimer(meta);
    else {
      this.lifecycle()?.settle(meta.correlation, ACTION_FEEDBACK_STATE.REFUSED);
      this.finishLocal(meta.correlation, 'refused', refusalReason);
    }
    return accepted;
  }

  remember(meta, outcome, reason) {
    this.localTerminals.set(gmActionEntryKey(meta.operatorId, meta.correlation), { ...meta, outcome, reason });
    while (this.localTerminals.size > this.capacity) {
      this.localTerminals.delete(this.localTerminals.keys().next().value);
    }
  }

  finishLocal(correlation, outcome, reason) {
    const meta = this.get(correlation);
    if (!meta) return false;
    this.clearTimer(meta);
    this.pending.delete(correlation);
    this.remember(meta, outcome, reason);
    this.onLocalTerminal(meta, outcome, reason);
    return true;
  }

  settle(result, accepts = () => true) {
    const meta = this.get(result.correlation);
    if (!meta || meta.operatorId !== result.operator_id || !accepts(meta, result)) return null;
    this.clearTimer(meta);
    this.pending.delete(result.correlation);
    const state = result.outcome === 'refused' ? ACTION_FEEDBACK_STATE.REFUSED : ACTION_FEEDBACK_STATE.APPLIED;
    this.lifecycle()?.settle(result.correlation, state);
    return { meta, state };
  }

  replace(results, { accepts, onSettled = () => {}, onUnmatched = () => {} } = {}) {
    this.authoritative = results.slice(-this.capacity);
    this.localTerminals.clear();
    // Settlement uses the full feed, including results outside display capacity.
    for (const result of results) {
      const match = this.settle(result, accepts);
      if (match) onSettled(match.meta, result, match.state);
      else onUnmatched(result);
    }
  }

  *entries() {
    const keys = new Set();
    for (const result of this.authoritative) {
      keys.add(gmActionEntryKey(result.operator_id, result.correlation));
      yield { kind: 'result', value: result };
    }
    for (const [key, terminal] of this.localTerminals) {
      if (!keys.has(key)) yield { kind: 'local', value: terminal };
    }
    for (const meta of this.values()) {
      if (!keys.has(gmActionEntryKey(meta.operatorId, meta.correlation))) yield { kind: 'pending', value: meta };
    }
  }

  reset(cancel = true) {
    for (const meta of this.values()) this.clearTimer(meta);
    if (cancel) for (const meta of [...this.values()]) this.lifecycle()?.cancel(meta.correlation);
    this.pending.clear();
    this.localTerminals.clear();
    this.authoritative = [];
  }
}

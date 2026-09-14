import { DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS, isValidActionCorrelation } from './action-feedback.js';

/** Adapter for typed GM families predating the shared semantic lifecycle.
 * Tracks only this document's submitted correlations, never journal history. */
export function createPrivateRequestFeedback({ audio, getOperator,
  schedule = setTimeout, cancelSchedule = clearTimeout,
} = {}) {
  const pending = new Map();
  const key = (operator, correlation) => `${operator}\0${correlation}`;
  const emit = (record, state) => audio?.action({ actionId: record.actionId,
    correlation: record.correlation, state, lifecycleTransition: true });
  function finish(id, state) {
    const record = pending.get(id);
    if (!record) return false;
    pending.delete(id); cancelSchedule(record.timer); emit(record, state);
    return true;
  }
  function submit(actionId, request, send) {
    if (!audio) return send();
    const operator = request?.operator_id || getOperator()?.id;
    if (!operator || !isValidActionCorrelation(request?.correlation)) return send();
    const id = key(operator, request.correlation);
    if (pending.has(id)) return send();
    const record = { actionId, correlation: request.correlation, timer: null };
    emit(record, 'Pressed');
    let result;
    try { result = send(); } catch (_) { emit(record, 'Refused'); return false; }
    if (result === false) { emit(record, 'Refused'); return result; }
    // Bounded in-flight correlation state, retired at terminal/timeout/reset.
    if (pending.size >= 128) finish(pending.keys().next().value, 'TimedOut');
    pending.set(id, record); emit(record, 'Pending');
    record.timer = schedule(() => finish(id, 'TimedOut'), DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS);
    return result;
  }
  function settle(rows) {
    if (!Array.isArray(rows)) return;
    for (const row of rows) {
      if (!row || !['applied', 'no-op', 'refused'].includes(row.outcome)) continue;
      finish(key(row.operator_id, row.correlation), row.outcome === 'refused' ? 'Refused' : 'Applied');
    }
  }
  return { submit, settle, reset() {
    for (const record of pending.values()) cancelSchedule(record.timer);
    pending.clear();
  } };
}

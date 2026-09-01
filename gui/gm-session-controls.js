/** Accessible, authoritative GM session pause/result presentation (#1292). */

import {
  ACTION_FEEDBACK_STATE,
  ActionFeedbackLifecycle,
  DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS,
  emitActionFeedbackTransition,
} from './action-feedback.js';
import { createSemanticActionRegistry } from './semantic-action-registry.js';
import {
  GM_ACTION_CONTEXT,
  GM_PAUSE_ACTION_ID,
  GM_RESUME_ACTION_ID,
  registerGmSessionActions,
} from './gm-session-actions.js';

export const GM_SESSION_FEED_CAPACITY = 32;

const RESULT_OUTCOMES = new Set(['applied', 'no-op', 'refused']);
const LOCAL_INGRESS_REFUSAL = 'ingress-rejected';

/** Rust wire identities stay machine-readable while their copy is localised. */
export const GM_ACTION_REFUSAL_REASON_LABELS = Object.freeze({
  'not-in-fleet': 'server.gm.session.reason.not_in_fleet',
  'not-game-master': 'server.gm.session.reason.not_game_master',
  'operator-mismatch': 'server.gm.session.reason.operator_mismatch',
  'invalid-operator': 'server.gm.session.reason.invalid_operator',
  'origin-mismatch': 'server.gm.session.reason.origin_mismatch',
  'conflicting-grant': 'server.gm.session.reason.conflicting_grant',
  'non-contiguous-sequence': 'server.gm.session.reason.non_contiguous_sequence',
  'journal-full': 'server.gm.session.reason.journal_full',
  'unreadable-request': 'server.gm.session.reason.unreadable_request',
  'wrong-phase': 'server.gm.session.reason.wrong_phase',
});

function parseResult(value) {
  if (!value || typeof value !== 'object'
      || typeof value.operator_id !== 'string' || value.operator_id.length === 0
      || typeof value.correlation !== 'string' || value.correlation.length === 0
      || typeof value.requested_active !== 'boolean'
      || !RESULT_OUTCOMES.has(value.outcome)
      || !Number.isSafeInteger(value.tick) || value.tick < 0
      || (value.reason != null && typeof value.reason !== 'string')) return null;
  return {
    operator_id: value.operator_id,
    correlation: value.correlation,
    requested_active: value.requested_active,
    outcome: value.outcome,
    tick: value.tick,
    ...(value.reason ? { reason: value.reason } : {}),
  };
}

/** Parse one absolute Host Channel projection without retaining partial data. */
export function parseGmSessionPayload(payload) {
  let value = payload;
  if (typeof value === 'string') {
    try { value = JSON.parse(value); } catch (_) { return undefined; }
  }
  if (!value || typeof value !== 'object' || typeof value.paused !== 'boolean'
      || !Array.isArray(value.results)) return undefined;
  const results = [];
  for (const candidate of value.results) {
    const parsed = parseResult(candidate);
    if (!parsed) return undefined;
    results.push(parsed);
  }
  return { paused: value.paused, results };
}

function setAccessibility(region, heading, pause, resume, state, feedback, log) {
  if (region) {
    region.setAttribute('role', 'region');
    if (heading) region.setAttribute('aria-labelledby', heading.id);
  }
  for (const button of [pause, resume]) {
    if (!button) continue;
    button.type = 'button';
    if (state && feedback) button.setAttribute('aria-describedby', `${state.id} ${feedback.id}`);
  }
  if (state) {
    state.setAttribute('role', 'status');
    state.setAttribute('aria-live', 'polite');
    state.setAttribute('aria-atomic', 'true');
  }
  if (feedback) {
    feedback.setAttribute('role', 'status');
    feedback.setAttribute('aria-live', 'polite');
    feedback.setAttribute('aria-atomic', 'true');
  }
  if (log) {
    log.setAttribute('role', 'log');
    log.setAttribute('aria-live', 'polite');
    log.setAttribute('aria-relevant', 'additions text');
  }
}

function entryKey(operatorId, correlation) {
  return JSON.stringify([operatorId, correlation]);
}

/**
 * Mount the GM session controls over injected page/transport seams.
 *
 * Pending is deliberately confined to feedback/log presentation. Only
 * `update()` writes `paused`, the state sentence, or authoritative data attrs.
 * A supplied `actions` registry is the host page's shared registry; this
 * module registers its GM adapters into it but never installs another input
 * listener.
 */
export function createGmSessionControls({
  doc = globalThis.document,
  win = doc && doc.defaultView,
  t = (id) => id,
  submitSessionPaused = null,
  getOperator = () => null,
  getOperatorName = (id) => id,
  correlation,
  now,
  capacity = GM_SESSION_FEED_CAPACITY,
  timeoutMs = DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS,
  schedule = (fn, delay) => setTimeout(fn, delay),
  cancelSchedule = (timer) => clearTimeout(timer),
  defer = (fn) => queueMicrotask(fn),
  actions: suppliedActions = null,
  actionFeedback: suppliedActionFeedback = null,
} = {}) {
  const region = doc && doc.getElementById('gm-session-controls');
  const heading = doc && doc.getElementById('gm-session-heading');
  const pause = doc && doc.getElementById('gm-session-pause');
  const resume = doc && doc.getElementById('gm-session-resume');
  const state = doc && doc.getElementById('gm-session-state');
  const feedbackStatus = doc && doc.getElementById('gm-session-feedback');
  const log = doc && doc.getElementById('gm-session-log');
  const boundedCapacity = Math.max(
    1,
    Number.isInteger(capacity) ? capacity : GM_SESSION_FEED_CAPACITY,
  );
  const boundedTimeoutMs = Number.isFinite(timeoutMs) && timeoutMs >= 0
    ? timeoutMs : DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS;

  setAccessibility(region, heading, pause, resume, state, feedbackStatus, log);
  if (pause) {
    pause.textContent = t('semantic_action.gm.session_pause.label');
    pause.setAttribute('aria-label', t('semantic_action.gm.session_pause.accessibility'));
  }
  if (resume) {
    resume.textContent = t('semantic_action.gm.session_resume.label');
    resume.setAttribute('aria-label', t('semantic_action.gm.session_resume.accessibility'));
  }

  let paused = null;
  let authoritativeResults = [];
  const pending = new Map();
  const localTerminals = new Map();
  const rows = new Map();
  let actions = suppliedActions;
  let actionFeedback = suppliedActionFeedback;

  function operator() {
    try {
      const value = typeof getOperator === 'function' ? getOperator() : null;
      return value && typeof value.id === 'string' && value.id.length > 0 ? value : null;
    } catch (_) {
      return null;
    }
  }

  function operatorName(id) {
    try {
      const value = typeof getOperatorName === 'function' ? getOperatorName(id) : id;
      return typeof value === 'string' && value.length > 0 ? value : id;
    } catch (_) {
      return id;
    }
  }

  function actionLabel(actionId) {
    const definition = actions && actions.action(actionId);
    return definition ? t(definition.labelId) : actionId;
  }

  function requestedState(active) {
    return t(active ? 'server.gm.session.requested_paused' : 'server.gm.session.requested_running');
  }

  function refusalText(reason) {
    if (!reason) return t('server.gm.session.reason_unspecified');
    if (reason === LOCAL_INGRESS_REFUSAL) {
      return t('server.gm.session.reason.ingress_rejected');
    }
    const labelId = GM_ACTION_REFUSAL_REASON_LABELS[reason];
    return labelId
      ? t(labelId)
      : t('server.gm.session.reason_unknown', { reason });
  }

  function clearPendingTimer(meta) {
    if (!meta || meta.timerScheduled !== true) return;
    try { cancelSchedule(meta.timer); } catch (_) { /* timer already completed */ }
    meta.timer = null;
    meta.timerScheduled = false;
  }

  function appendRow(operatorId, correlationValue) {
    if (!log) return null;
    const key = entryKey(operatorId, correlationValue);
    const row = doc.createElement('li');
    row.className = 'gm-session-log-entry';
    row.dataset.entryKey = key;
    row.dataset.operatorId = operatorId;
    row.dataset.correlation = correlationValue;
    rows.set(key, row);
    log.appendChild(row);
    return row;
  }

  function paintPendingRow(meta) {
    const row = appendRow(meta.operatorId, meta.correlation);
    if (!row) return;
    row.dataset.outcome = 'pending';
    row.textContent = t('server.gm.session.result_pending', {
      name: meta.operatorName,
      state: requestedState(meta.active),
      correlation: meta.correlation,
    });
  }

  function resultText(result) {
    const suffix = result.outcome === 'no-op' ? 'no_op' : result.outcome;
    return t(`server.gm.session.result_${suffix}`, {
      name: operatorName(result.operator_id),
      state: requestedState(result.requested_active),
      tick: String(result.tick),
      correlation: result.correlation,
      reason: refusalText(result.reason),
    });
  }

  function paintResultRow(result) {
    const row = appendRow(result.operator_id, result.correlation);
    if (!row) return;
    row.dataset.outcome = result.outcome;
    row.dataset.tick = String(result.tick);
    if (result.reason) row.dataset.reason = result.reason;
    row.textContent = resultText(result);
  }

  function paintLocalTerminalRow(terminal) {
    const row = appendRow(terminal.operatorId, terminal.correlation);
    if (!row) return;
    row.dataset.outcome = terminal.outcome;
    if (terminal.reason) row.dataset.reason = terminal.reason;
    const statusId = terminal.outcome === 'timed-out'
      ? 'server.gm.session.result_timed_out'
      : 'server.gm.session.result_refused_local';
    row.textContent = t(statusId, {
      name: terminal.operatorName,
      state: requestedState(terminal.active),
      correlation: terminal.correlation,
      reason: refusalText(terminal.reason),
    });
  }

  /** Rebuild deterministically: absolute terminal order, then local live rows. */
  function renderLog() {
    if (!log) return;
    log.replaceChildren();
    rows.clear();
    const authoritativeKeys = new Set();
    for (const result of authoritativeResults) {
      authoritativeKeys.add(entryKey(result.operator_id, result.correlation));
      paintResultRow(result);
    }
    for (const [key, terminal] of localTerminals) {
      if (!authoritativeKeys.has(key)) paintLocalTerminalRow(terminal);
    }
    for (const meta of pending.values()) {
      const key = entryKey(meta.operatorId, meta.correlation);
      if (!authoritativeKeys.has(key)) paintPendingRow(meta);
    }
  }

  function rememberLocalTerminal(meta, outcome, reason) {
    const key = entryKey(meta.operatorId, meta.correlation);
    localTerminals.set(key, {
      operatorId: meta.operatorId,
      operatorName: meta.operatorName,
      correlation: meta.correlation,
      active: meta.active,
      outcome,
      reason,
    });
    while (localTerminals.size > boundedCapacity) {
      localTerminals.delete(localTerminals.keys().next().value);
    }
  }

  function finishLocalPending(correlationValue, outcome, reason) {
    const meta = pending.get(correlationValue);
    if (!meta) return false;
    clearPendingTimer(meta);
    pending.delete(correlationValue);
    rememberLocalTerminal(meta, outcome, reason);
    renderLog();
    return true;
  }

  function paintFeedback(value) {
    if (!feedbackStatus || value.isCurrent === false) return;
    feedbackStatus.dataset.state = value.state || '';
    feedbackStatus.textContent = value.statusId
      ? t('action_feedback.summary', {
          action: actionLabel(value.actionId),
          status: t(value.statusId),
        })
      : '';
  }

  function handleFeedbackTransition(value) {
    if (!value || (value.actionId !== GM_PAUSE_ACTION_ID
        && value.actionId !== GM_RESUME_ACTION_ID)) return;
    paintFeedback(value);
    const meta = pending.get(value.correlation);
    if (value.state === ACTION_FEEDBACK_STATE.PENDING && meta) {
      renderLog();
      if (meta.localReason) {
        // Let the lifecycle finish publishing Pending before its terminal
        // refusal. Settling re-entrantly from an event listener would make
        // later listeners observe Refused before the outer Pending event.
        if (!meta.refusalScheduled) {
          meta.refusalScheduled = true;
          defer(() => actionFeedback.settle(
            value.correlation,
            ACTION_FEEDBACK_STATE.REFUSED,
          ));
        }
        return;
      }
      if (!meta.timerScheduled) {
        meta.timerScheduled = true;
        meta.timer = schedule(() => {
          if (!actionFeedback.settle(meta.correlation, ACTION_FEEDBACK_STATE.TIMED_OUT)) {
            finishLocalPending(meta.correlation, 'timed-out', null);
          }
        }, boundedTimeoutMs);
      }
    } else if (value.state === ACTION_FEEDBACK_STATE.TIMED_OUT && meta) {
      finishLocalPending(value.correlation, 'timed-out', null);
    } else if (value.state === ACTION_FEEDBACK_STATE.REFUSED && meta && meta.localReason) {
      finishLocalPending(value.correlation, 'refused', meta.localReason);
    }
  }

  const onFeedbackEvent = (event) => handleFeedbackTransition(event && event.detail);
  if (suppliedActionFeedback && win && typeof win.addEventListener === 'function') {
    win.addEventListener('phoenix-action-feedback', onFeedbackEvent);
  }
  if (!actionFeedback) {
    actionFeedback = new ActionFeedbackLifecycle({
      ...(typeof correlation === 'function' ? { correlation } : {}),
      ...(typeof now === 'function' ? { now } : {}),
      capacity: boundedCapacity,
      onTransition: (value) => {
        handleFeedbackTransition(value);
        emitActionFeedbackTransition(win, value);
      },
    });
  }
  if (!actions) actions = createSemanticActionRegistry({ actionFeedback });
  const pauseDefinition = actions.action(GM_PAUSE_ACTION_ID);
  const resumeDefinition = actions.action(GM_RESUME_ACTION_ID);
  if (!!pauseDefinition !== !!resumeDefinition) {
    throw new Error('shared host registry has a partial GM session action set');
  }

  function expireOldestPending() {
    const oldest = pending.keys().next().value;
    if (!oldest) return;
    if (!actionFeedback.settle(oldest, ACTION_FEEDBACK_STATE.TIMED_OUT)) {
      finishLocalPending(oldest, 'timed-out', null);
    }
  }

  function submit(active, correlationValue) {
    const current = operator();
    if (!current) return false;
    while (pending.size >= boundedCapacity) expireOldestPending();
    const meta = {
      active,
      correlation: correlationValue,
      operatorId: current.id,
      operatorName: typeof current.name === 'string' && current.name.length > 0
        ? current.name : operatorName(current.id),
      timer: null,
      timerScheduled: false,
      localReason: null,
      refusalScheduled: false,
    };
    pending.set(correlationValue, meta);
    let accepted = false;
    try {
      accepted = typeof submitSessionPaused === 'function'
        && submitSessionPaused(active, correlationValue) !== false;
    } catch (_) {
      accepted = false;
    }
    if (!accepted) meta.localReason = LOCAL_INGRESS_REFUSAL;
    // A synchronous refusal is still a handled semantic action: the shared
    // lifecycle will publish Pending then immediately Refused, never silently
    // cancel the operator's accessible result.
    return true;
  }

  if (!pauseDefinition) registerGmSessionActions(actions, { submitSessionPaused: submit });

  function paintAuthoritativeState() {
    for (const [button, active] of [[pause, paused], [resume, paused == null ? null : !paused]]) {
      if (!button) continue;
      if (active == null) button.removeAttribute('aria-pressed');
      else button.setAttribute('aria-pressed', String(active));
    }
    if (state) {
      state.dataset.paused = paused == null ? '' : String(paused);
      state.textContent = paused == null
        ? t('server.gm.session.state_waiting')
        : t(paused ? 'server.gm.session.state_paused' : 'server.gm.session.state_running');
    }
  }

  function refreshAdmission() {
    const admitted = !!operator();
    if (region) region.dataset.admitted = String(admitted);
    for (const button of [pause, resume]) {
      if (!button) continue;
      button.disabled = !admitted;
      button.setAttribute('aria-disabled', admitted ? 'false' : 'true');
    }
    return admitted;
  }

  function activate(actionId, source = 'control') {
    if (!refreshAdmission()) {
      return { claimed: true, actionId, handled: false };
    }
    return actions.activate(actionId, { context: GM_ACTION_CONTEXT, source });
  }

  function update(payload) {
    const projection = parseGmSessionPayload(payload);
    if (!projection) return false;
    paused = projection.paused;
    authoritativeResults = projection.results.slice(-boundedCapacity);
    localTerminals.clear();
    // Settle every exact local occurrence even if the display capacity trims
    // it. A same-correlation result attributed to another GM is never ours.
    for (const result of projection.results) {
      const meta = pending.get(result.correlation);
      if (!meta || meta.operatorId !== result.operator_id) continue;
      clearPendingTimer(meta);
      pending.delete(result.correlation);
      actionFeedback.settle(
        result.correlation,
        result.outcome === 'refused'
          ? ACTION_FEEDBACK_STATE.REFUSED
          : ACTION_FEEDBACK_STATE.APPLIED,
      );
    }
    paintAuthoritativeState();
    renderLog();
    refreshAdmission();
    return true;
  }

  /** Explicit run boundary, called from the authoritative Lobby transition. */
  function reset() {
    for (const meta of pending.values()) clearPendingTimer(meta);
    for (const correlationValue of [...pending.keys()]) actionFeedback.cancel(correlationValue);
    pending.clear();
    localTerminals.clear();
    authoritativeResults = [];
    paused = null;
    rows.clear();
    if (log) log.replaceChildren();
    if (feedbackStatus) {
      feedbackStatus.dataset.state = '';
      feedbackStatus.textContent = '';
    }
    paintAuthoritativeState();
    refreshAdmission();
  }

  const onPause = (event) => {
    if (event && typeof event.preventDefault === 'function') event.preventDefault();
    activate(GM_PAUSE_ACTION_ID);
  };
  const onResume = (event) => {
    if (event && typeof event.preventDefault === 'function') event.preventDefault();
    activate(GM_RESUME_ACTION_ID);
  };
  if (pause) pause.addEventListener('click', onPause);
  if (resume) resume.addEventListener('click', onResume);
  paintAuthoritativeState();
  refreshAdmission();

  function destroy() {
    if (pause) pause.removeEventListener('click', onPause);
    if (resume) resume.removeEventListener('click', onResume);
    if (suppliedActionFeedback && win && typeof win.removeEventListener === 'function') {
      win.removeEventListener('phoenix-action-feedback', onFeedbackEvent);
    }
    for (const meta of pending.values()) clearPendingTimer(meta);
    pending.clear();
  }

  return {
    actions,
    actionFeedback,
    activate,
    update,
    reset,
    refreshAdmission,
    state: () => ({
      paused,
      pending: pending.size,
      entries: rows.size,
      authoritative: authoritativeResults.length,
    }),
    destroy,
  };
}

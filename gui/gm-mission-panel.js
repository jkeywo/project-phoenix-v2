/**
 * The GM mission panel: authored GM-operable events and their Fire and Pause
 * controls (issues #1301, #1302 and #1303, PRD #930 milestone M2).
 *
 * Pure over injected page/transport seams, like every other GM surface here —
 * no globals, no WASM, no timers of its own beyond the injected scheduler — so
 * vitest drives the whole lifecycle against a plain DOM.
 *
 * # Why this is not the Session controls with a longer list
 *
 * `gm-session-controls.js` presents a FIXED pair of semantic actions (Pause,
 * Resume) that are keybound and live in the shared host action registry. The
 * controllable-event set is authored by the scenario: it has no fixed
 * cardinality, no fixed ids, and nothing to bind a key to. So this module owns
 * its own correlations and mints one per press, while reusing the two things
 * that genuinely are shared — the authoritative feedback lifecycle
 * (`action-feedback.js`) and the one refusal-reason table
 * (`gm-action-reasons.js`).
 *
 * # The projection is absolute
 *
 * `update()` replaces the event list and the result feed outright. A GM that
 * reconnects, or whose peer restored from a snapshot, therefore sees exactly
 * what a live one sees; nothing here accumulates state the simulation owns.
 * The only local state is the operator's own in-flight presses, which exist to
 * keep a press accessible between the click and the authoritative answer.
 */

import {
  ACTION_FEEDBACK_STATE,
  ActionFeedbackLifecycle,
  DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS,
  createActionCorrelation,
} from './action-feedback.js';
import {
  GM_ACTION_REFUSAL_REASON_LABELS,
  LOCAL_INGRESS_REFUSAL,
} from './gm-action-reasons.js';

export const GM_MISSION_FEED_CAPACITY = 32;

/** The semantic-action id family a Fire press reports feedback under. */
export const GM_FIRE_ACTION_PREFIX = 'gm.mission.fire:';

/** The same, for a Pause/Resume press (issue #1303). */
export const GM_PAUSE_ACTION_PREFIX = 'gm.mission.pause:';

const RESULT_OUTCOMES = new Set(['applied', 'no-op', 'refused']);

/**
 * The levers of the event-control family, as the durable result spells them.
 *
 * Required on every result rather than optional the way `target` is, and the
 * difference is what each one decides: `target` is a name INSIDE a sentence,
 * while the verb decides WHICH sentence — and there is no honest neutral
 * wording for "a Game Master did something to this event". A payload without it
 * is a producer bug; freezing this one bounded feed until the entry rotates out
 * is better than telling every GM that a Resume was a Fire.
 */
const RESULT_VERBS = new Set(['fire', 'pause']);

/** The shared lifecycle's states, spelled in this surface's status sentence. */
const FEEDBACK_STATUS_IDS = Object.freeze({
  [ACTION_FEEDBACK_STATE.PENDING]: 'action_feedback.pending',
  [ACTION_FEEDBACK_STATE.APPLIED]: 'action_feedback.applied',
  [ACTION_FEEDBACK_STATE.REFUSED]: 'action_feedback.refused',
  [ACTION_FEEDBACK_STATE.TIMED_OUT]: 'action_feedback.timed_out',
});

function parseEvent(value) {
  if (!value || typeof value !== 'object'
      || typeof value.id !== 'string' || value.id.length === 0
      || typeof value.label !== 'string' || value.label.length === 0
      || typeof value.fire !== 'boolean' || typeof value.pause !== 'boolean'
      || typeof value.skip !== 'boolean' || typeof value.repeatable !== 'boolean'
      || typeof value.spent !== 'boolean' || typeof value.armed !== 'boolean'
      || typeof value.paused !== 'boolean') return null;
  return {
    id: value.id,
    label: value.label,
    fire: value.fire,
    pause: value.pause,
    skip: value.skip,
    repeatable: value.repeatable,
    spent: value.spent,
    armed: value.armed,
    paused: value.paused,
  };
}

function parseResult(value) {
  if (!value || typeof value !== 'object'
      || typeof value.operator_id !== 'string' || value.operator_id.length === 0
      || typeof value.correlation !== 'string' || value.correlation.length === 0
      || !RESULT_OUTCOMES.has(value.outcome)
      || !Number.isSafeInteger(value.tick) || value.tick < 0
      || (value.reason != null && typeof value.reason !== 'string')
      || (value.target != null && typeof value.target !== 'string')
      || !RESULT_VERBS.has(value.verb)
      || typeof value.requested_active !== 'boolean') return null;
  return {
    operator_id: value.operator_id,
    correlation: value.correlation,
    outcome: value.outcome,
    tick: value.tick,
    verb: value.verb,
    requested_active: value.requested_active,
    ...(value.reason ? { reason: value.reason } : {}),
    ...(value.target ? { target: value.target } : {}),
  };
}

/**
 * Parse one absolute Host Channel projection without retaining partial data.
 *
 * A malformed row rejects the WHOLE payload rather than being skipped: a panel
 * that quietly drops one event would offer a GM an incomplete set of levers
 * with no indication that it had done so.
 */
export function parseGmMissionPayload(payload) {
  let value = payload;
  if (typeof value === 'string') {
    try { value = JSON.parse(value); } catch (_) { return undefined; }
  }
  if (!value || typeof value !== 'object'
      || !Array.isArray(value.events) || !Array.isArray(value.results)) return undefined;
  const events = [];
  for (const candidate of value.events) {
    const parsed = parseEvent(candidate);
    if (!parsed) return undefined;
    events.push(parsed);
  }
  const results = [];
  for (const candidate of value.results) {
    const parsed = parseResult(candidate);
    if (!parsed) return undefined;
    results.push(parsed);
  }
  return { events, results };
}

/** Whether a projected event can accept a Fire from this operator right now. */
export function eventIsFireable(event) {
  return !!event && event.fire === true && event.spent !== true;
}

/**
 * Whether a projected event can accept a Pause/Resume right now (issue #1303).
 *
 * Deliberately NOT gated on `spent`, unlike Fire: pausing decides whether the
 * automatic condition is evaluated at all, and a spent one-shot whose condition
 * still runs is exactly the case a GM might want quiet. The server's
 * `pausable_index` makes the same call, so the disabled state here and the
 * refusal there agree.
 */
export function eventIsPausable(event) {
  return !!event && event.pause === true;
}

/** The localized past-tense verb one result sentence is built around. */
function verbTextId(verb, active) {
  if (verb !== 'pause') return 'server.gm.mission.verb_fire';
  return active ? 'server.gm.mission.verb_pause' : 'server.gm.mission.verb_resume';
}

function setAccessibility(region, heading, list, empty, feedback, log) {
  if (region) {
    region.setAttribute('role', 'region');
    if (heading) region.setAttribute('aria-labelledby', heading.id);
  }
  if (list) list.setAttribute('role', 'list');
  if (empty) {
    empty.setAttribute('role', 'status');
    empty.setAttribute('aria-live', 'polite');
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

/** Mount the GM mission panel over injected page/transport seams. */
export function createGmMissionPanel({
  doc = globalThis.document,
  win = doc && doc.defaultView,
  t = (id) => id,
  submitFireEvent = null,
  submitSetEventPaused = null,
  getOperator = () => null,
  getOperatorName = (id) => id,
  correlation = createActionCorrelation,
  now,
  capacity = GM_MISSION_FEED_CAPACITY,
  timeoutMs = DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS,
  schedule = (fn, delay) => setTimeout(fn, delay),
  cancelSchedule = (timer) => clearTimeout(timer),
  actionFeedback: suppliedActionFeedback = null,
} = {}) {
  const region = doc && doc.getElementById('gm-mission-panel');
  const heading = doc && doc.getElementById('gm-mission-heading');
  const list = doc && doc.getElementById('gm-mission-events');
  const empty = doc && doc.getElementById('gm-mission-empty');
  const feedbackStatus = doc && doc.getElementById('gm-mission-feedback');
  const log = doc && doc.getElementById('gm-mission-log');
  const boundedCapacity = Math.max(
    1,
    Number.isInteger(capacity) ? capacity : GM_MISSION_FEED_CAPACITY,
  );
  const boundedTimeoutMs = Number.isFinite(timeoutMs) && timeoutMs >= 0
    ? timeoutMs : DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS;

  setAccessibility(region, heading, list, empty, feedbackStatus, log);

  let events = [];
  let authoritativeResults = [];
  const pending = new Map();
  const localTerminals = new Map();
  // eventId -> { fire, pause } — one entry per rendered lever, so admission
  // refresh and teardown reach both without a second registry.
  const buttons = new Map();

  const actionFeedback = suppliedActionFeedback || new ActionFeedbackLifecycle({
    ...(typeof correlation === 'function' ? { correlation } : {}),
    ...(typeof now === 'function' ? { now } : {}),
    capacity: boundedCapacity,
  });

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

  function refusalText(reason) {
    if (!reason) return t('server.gm.mission.reason_unspecified');
    if (reason === LOCAL_INGRESS_REFUSAL) {
      return t('server.gm.session.reason.ingress_rejected');
    }
    const labelId = GM_ACTION_REFUSAL_REASON_LABELS[reason];
    return labelId ? t(labelId) : t('server.gm.mission.reason_unknown', { reason });
  }

  function clearPendingTimer(meta) {
    if (!meta || meta.timerScheduled !== true) return;
    try { cancelSchedule(meta.timer); } catch (_) { /* timer already completed */ }
    meta.timer = null;
    meta.timerScheduled = false;
  }

  function appendRow(operatorId, correlationValue) {
    if (!log) return null;
    const row = doc.createElement('li');
    row.className = 'gm-mission-log-entry';
    row.dataset.entryKey = entryKey(operatorId, correlationValue);
    row.dataset.operatorId = operatorId;
    row.dataset.correlation = correlationValue;
    log.appendChild(row);
    return row;
  }

  function paintResultRow(result) {
    const row = appendRow(result.operator_id, result.correlation);
    if (!row) return;
    const suffix = result.outcome === 'no-op' ? 'no_op' : result.outcome;
    row.dataset.outcome = result.outcome;
    row.dataset.tick = String(result.tick);
    row.dataset.verb = result.verb;
    if (result.target) row.dataset.event = result.target;
    if (result.reason) row.dataset.reason = result.reason;
    row.textContent = t(`server.gm.mission.result_${suffix}`, {
      name: operatorName(result.operator_id),
      verb: t(verbTextId(result.verb, result.requested_active)),
      event: result.target || '',
      tick: String(result.tick),
      correlation: result.correlation,
      reason: refusalText(result.reason),
    });
  }

  function paintLocalRow(meta, outcome, reason) {
    const row = appendRow(meta.operatorId, meta.correlation);
    if (!row) return;
    row.dataset.outcome = outcome;
    row.dataset.event = meta.event;
    row.dataset.verb = meta.verb;
    if (reason) row.dataset.reason = reason;
    const statusId = {
      pending: 'server.gm.mission.result_pending',
      'timed-out': 'server.gm.mission.result_timed_out',
      refused: 'server.gm.mission.result_refused_local',
    }[outcome];
    row.textContent = t(statusId, {
      name: meta.operatorName,
      verb: t(verbTextId(meta.verb, meta.active)),
      event: meta.event,
      correlation: meta.correlation,
      reason: refusalText(reason),
    });
  }

  /** Rebuild deterministically: absolute terminal order, then local live rows. */
  function renderLog() {
    if (!log) return;
    log.replaceChildren();
    const authoritativeKeys = new Set();
    for (const result of authoritativeResults) {
      authoritativeKeys.add(entryKey(result.operator_id, result.correlation));
      paintResultRow(result);
    }
    for (const [key, terminal] of localTerminals) {
      if (!authoritativeKeys.has(key)) {
        paintLocalRow(terminal, terminal.outcome, terminal.reason);
      }
    }
    for (const meta of pending.values()) {
      if (!authoritativeKeys.has(entryKey(meta.operatorId, meta.correlation))) {
        paintLocalRow(meta, 'pending', null);
      }
    }
  }

  /** The accessible name of one lever, spoken exactly as its button reads. */
  function controlAccessibility(verb, active, label) {
    if (verb === 'pause') {
      return t(
        active ? 'server.gm.mission.pause_accessibility' : 'server.gm.mission.resume_accessibility',
        { label },
      );
    }
    return t('server.gm.mission.fire_accessibility', { label });
  }

  function paintFeedback(state, event, verb = 'fire', active = true) {
    if (!feedbackStatus) return;
    feedbackStatus.dataset.state = state || '';
    feedbackStatus.dataset.event = event || '';
    feedbackStatus.dataset.verb = state ? verb : '';
    const statusId = FEEDBACK_STATUS_IDS[state];
    // The live region names the event the way the button beside it does: the
    // authored label, localized, under the lever that was pressed.
    // `fire_accessibility` and its Pause/Resume twins document {label} as that
    // label, and a screen reader hearing the qualified id here while reading
    // the label there would be listening to two different events. The id
    // survives only as the fallback for an event the projection has since
    // dropped (a layer unloaded under a pending press), where no label is left
    // to read, and in `data-event`, which stays machine-readable.
    const listed = events.find((candidate) => candidate.id === event);
    feedbackStatus.textContent = statusId
      ? t('action_feedback.summary', {
          action: controlAccessibility(verb, active, listed ? t(listed.label) : event || ''),
          status: t(statusId),
        })
      : '';
  }

  function rememberLocalTerminal(meta, outcome, reason) {
    localTerminals.set(entryKey(meta.operatorId, meta.correlation), {
      ...meta,
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
    paintFeedback(
      outcome === 'timed-out' ? ACTION_FEEDBACK_STATE.TIMED_OUT : ACTION_FEEDBACK_STATE.REFUSED,
      meta.event,
      meta.verb,
      meta.active,
    );
    renderLog();
    refreshAdmission();
    return true;
  }

  /**
   * Whether this operator already has an unsettled press on this event's
   * `verb`.
   *
   * Scoped to the lever, not the event (issue #1303): Fire and Pause are
   * independent authoritative requests about one event, and a Fire waiting on
   * its answer is no reason to refuse a Pause of the same event -- pressing
   * both in one moment is exactly the "hold this back, then run it when I say"
   * gesture the contract is for.
   */
  function hasPendingFor(eventId, verb) {
    for (const meta of pending.values()) {
      if (meta.event === eventId && meta.verb === verb) return true;
    }
    return false;
  }

  /**
   * Submit one attributed press of one lever and hold it until an
   * authoritative answer settles it.
   *
   * The whole local lifecycle -- correlation minting, the bounded pending map,
   * the timeout, the synchronous ingress refusal -- is identical for Fire and
   * Pause, so it is written once. Only the eligibility predicate, the
   * semantic-action id, the transport seam and the request body differ.
   */
  function submitPress({ eventId, verb, active, eligible, prefix, submit, request }) {
    const current = operator();
    const event = events.find((candidate) => candidate.id === eventId);
    if (!current || !eligible(event) || hasPendingFor(eventId, verb)) return false;
    while (pending.size >= boundedCapacity) {
      const oldest = pending.keys().next().value;
      if (oldest === undefined) break;
      finishLocalPending(oldest, 'timed-out', null);
    }
    const pressed = actionFeedback.press(`${prefix}${eventId}`);
    const meta = {
      event: eventId,
      verb,
      active,
      correlation: pressed.correlation,
      operatorId: current.id,
      operatorName: typeof current.name === 'string' && current.name.length > 0
        ? current.name : operatorName(current.id),
      timer: null,
      timerScheduled: false,
    };
    pending.set(pressed.correlation, meta);
    let accepted = false;
    try {
      accepted = typeof submit === 'function'
        && submit({ ...request, correlation: pressed.correlation }) !== false;
    } catch (_) {
      accepted = false;
    }
    actionFeedback.pending(pressed.correlation);
    if (!accepted) {
      // A synchronous ingress refusal is still a handled press: the operator
      // gets an accessible terminal answer rather than a silent no-op.
      actionFeedback.settle(pressed.correlation, ACTION_FEEDBACK_STATE.REFUSED);
      finishLocalPending(pressed.correlation, 'refused', LOCAL_INGRESS_REFUSAL);
      return true;
    }
    meta.timerScheduled = true;
    meta.timer = schedule(() => {
      actionFeedback.settle(meta.correlation, ACTION_FEEDBACK_STATE.TIMED_OUT);
      finishLocalPending(meta.correlation, 'timed-out', null);
    }, boundedTimeoutMs);
    paintFeedback(ACTION_FEEDBACK_STATE.PENDING, eventId, verb, active);
    renderLog();
    refreshAdmission();
    return true;
  }

  function fire(eventId) {
    return submitPress({
      eventId,
      verb: 'fire',
      active: true,
      eligible: eventIsFireable,
      prefix: GM_FIRE_ACTION_PREFIX,
      submit: submitFireEvent,
      request: { event: eventId },
    });
  }

  /**
   * Ask for the ABSOLUTE paused state of one event (issue #1303).
   *
   * `active` is what the operator wants to be true, not a toggle: two GMs
   * pressing at once must not depend on arrival order for the state the event
   * ends up in, and the server reduces a request for the state it is already in
   * to a No-op.
   */
  function setPaused(eventId, active) {
    return submitPress({
      eventId,
      verb: 'pause',
      active: !!active,
      eligible: eventIsPausable,
      prefix: GM_PAUSE_ACTION_PREFIX,
      submit: submitSetEventPaused,
      request: { event: eventId, active: !!active },
    });
  }

  /**
   * The one status sentence a row shows.
   *
   * Paused sits at the top (issue #1303) because it is the standing state a GM
   * put the event in and can take it out of, and because it changes what every
   * other line means: a "Ready" event whose condition is not being evaluated
   * would be a lie by omission. The `data-spent` / `data-armed` attributes
   * beside it still report the lifecycle facts a test or a stylesheet needs.
   */
  function eventStatusId(event) {
    if (event.paused) return 'server.gm.mission.state_paused';
    if (!event.fire) return 'server.gm.mission.state_unavailable';
    if (event.spent) return 'server.gm.mission.state_spent';
    if (event.armed || hasPendingFor(event.id, 'fire')) return 'server.gm.mission.state_armed';
    return 'server.gm.mission.state_ready';
  }

  /**
   * One lever's button. Written once because Fire and Pause differ only in
   * their role, their two label ids and their click handler; the accessible
   * name, the id plumbing and the registry entry are the same contract.
   */
  function leverButton(event, role, textId, accessibleName, onClick) {
    const button = doc.createElement('button');
    button.type = 'button';
    button.dataset.role = role;
    button.dataset.eventId = event.id;
    button.textContent = t(textId);
    button.setAttribute('aria-label', accessibleName);
    button.addEventListener('click', onClick);
    return button;
  }

  function renderEvents() {
    if (!list) return;
    list.replaceChildren();
    buttons.clear();
    for (const event of events) {
      const row = doc.createElement('li');
      row.className = 'gm-mission-event';
      row.dataset.eventId = event.id;
      row.dataset.spent = String(event.spent);
      row.dataset.armed = String(event.armed);
      row.dataset.paused = String(event.paused);
      row.dataset.repeatable = String(event.repeatable);
      const label = doc.createElement('span');
      label.className = 'gm-mission-event-label';
      label.textContent = t(event.label);
      const status = doc.createElement('span');
      status.className = 'gm-mission-event-state';
      status.textContent = t(eventStatusId(event));
      row.append(label, status);
      const levers = {};
      if (event.fire) {
        levers.fire = leverButton(
          event,
          'fire',
          'server.gm.mission.fire',
          controlAccessibility('fire', true, t(event.label)),
          onFireClick,
        );
        row.append(levers.fire);
      }
      // Only an event that DECLARES Pause gets the toggle, and its one button
      // reads Pause or Resume from the absolute state rather than being two
      // controls: there is one lever with two positions, and rendering both
      // would invite a GM to press the one that is already true.
      if (event.pause) {
        levers.pause = leverButton(
          event,
          'pause',
          event.paused ? 'server.gm.mission.resume' : 'server.gm.mission.pause',
          controlAccessibility('pause', !event.paused, t(event.label)),
          onPauseClick,
        );
        levers.pause.dataset.requestedActive = String(!event.paused);
        row.append(levers.pause);
      }
      buttons.set(event.id, levers);
      list.append(row);
    }
    if (empty) {
      empty.hidden = events.length > 0;
      empty.textContent = events.length > 0 ? '' : t('server.gm.mission.empty');
    }
  }

  function refreshAdmission() {
    const admitted = !!operator();
    if (region) region.dataset.admitted = String(admitted);
    for (const [eventId, levers] of buttons) {
      const event = events.find((candidate) => candidate.id === eventId);
      if (levers.fire) {
        const enabled = admitted && eventIsFireable(event) && !hasPendingFor(eventId, 'fire');
        levers.fire.disabled = !enabled;
        levers.fire.setAttribute('aria-disabled', enabled ? 'false' : 'true');
      }
      if (levers.pause) {
        const enabled = admitted && eventIsPausable(event) && !hasPendingFor(eventId, 'pause');
        levers.pause.disabled = !enabled;
        levers.pause.setAttribute('aria-disabled', enabled ? 'false' : 'true');
      }
    }
    return admitted;
  }

  function onFireClick(event) {
    if (event && typeof event.preventDefault === 'function') event.preventDefault();
    const target = event && event.currentTarget;
    if (target && target.dataset && target.dataset.eventId) fire(target.dataset.eventId);
  }

  function onPauseClick(event) {
    if (event && typeof event.preventDefault === 'function') event.preventDefault();
    const target = event && event.currentTarget;
    if (!target || !target.dataset || !target.dataset.eventId) return;
    const listed = events.find((candidate) => candidate.id === target.dataset.eventId);
    // The absolute state the press asks for is read from the projection, not
    // from the button: the authoritative answer is what moves the toggle, so a
    // stale row can only ask for a state the server then reduces to a No-op.
    setPaused(target.dataset.eventId, !(listed && listed.paused));
  }

  function update(payload) {
    const projection = parseGmMissionPayload(payload);
    if (!projection) return false;
    events = projection.events;
    authoritativeResults = projection.results.slice(-boundedCapacity);
    localTerminals.clear();
    // Settle every exact local occurrence even if the display capacity trims
    // it. A same-correlation result attributed to another GM is never ours.
    for (const result of projection.results) {
      const meta = pending.get(result.correlation);
      if (!meta || meta.operatorId !== result.operator_id) continue;
      clearPendingTimer(meta);
      pending.delete(result.correlation);
      const state = result.outcome === 'refused'
        ? ACTION_FEEDBACK_STATE.REFUSED
        : ACTION_FEEDBACK_STATE.APPLIED;
      actionFeedback.settle(result.correlation, state);
      paintFeedback(state, meta.event, meta.verb, meta.active);
    }
    renderEvents();
    renderLog();
    refreshAdmission();
    return true;
  }

  /** Explicit run boundary, called from the authoritative Lobby transition. */
  function reset() {
    for (const meta of pending.values()) {
      clearPendingTimer(meta);
      actionFeedback.cancel(meta.correlation);
    }
    pending.clear();
    localTerminals.clear();
    authoritativeResults = [];
    events = [];
    renderEvents();
    if (log) log.replaceChildren();
    paintFeedback(null, null);
    refreshAdmission();
  }

  renderEvents();
  refreshAdmission();

  function destroy() {
    for (const levers of buttons.values()) {
      if (levers.fire) levers.fire.removeEventListener('click', onFireClick);
      if (levers.pause) levers.pause.removeEventListener('click', onPauseClick);
    }
    buttons.clear();
    for (const meta of pending.values()) clearPendingTimer(meta);
    pending.clear();
  }

  return {
    actionFeedback,
    fire,
    setPaused,
    update,
    reset,
    refreshAdmission,
    state: () => ({
      events: events.length,
      fireable: events.filter(eventIsFireable).length,
      pausable: events.filter(eventIsPausable).length,
      paused: events.filter((event) => event.paused).length,
      pending: pending.size,
      authoritative: authoritativeResults.length,
    }),
    destroy,
    win,
  };
}
